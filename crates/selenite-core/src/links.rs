//! Deterministic link identities and a pure, source-preserving rename planner.

use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt,
    path::{Path, PathBuf},
};

use crate::{Page, ParseError, WikiLink, validate_page_path};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveError {
    Invalid(String),
    Missing(String),
    Ambiguous {
        target: String,
        candidates: Vec<PathBuf>,
    },
}

impl fmt::Display for ResolveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(target) => write!(formatter, "invalid page link: {target}"),
            Self::Missing(target) => write!(formatter, "page link has no target: {target}"),
            Self::Ambiguous { target, candidates } => write!(
                formatter,
                "page link {target} matches {} pages",
                candidates.len()
            ),
        }
    }
}

impl Error for ResolveError {}

#[derive(Debug)]
pub enum LinkError {
    Parse(ParseError),
    DuplicatePath(PathBuf),
    MissingSource(PathBuf),
    DestinationExists(PathBuf),
    UnrepresentableTarget(PathBuf),
}

impl fmt::Display for LinkError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(error) => error.fmt(formatter),
            Self::DuplicatePath(path) => {
                write!(formatter, "duplicate page path: {}", path.display())
            }
            Self::MissingSource(path) => {
                write!(formatter, "rename source is missing: {}", path.display())
            }
            Self::DestinationExists(path) => write!(
                formatter,
                "rename destination already exists: {}",
                path.display()
            ),
            Self::UnrepresentableTarget(path) => write!(
                formatter,
                "path cannot be expressed safely as a wikilink: {}",
                path.display()
            ),
        }
    }
}

impl Error for LinkError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Parse(error) => Some(error),
            _ => None,
        }
    }
}

impl From<ParseError> for LinkError {
    fn from(value: ParseError) -> Self {
        Self::Parse(value)
    }
}

/// Folder-qualified links are relative to the vault root; `./` and `../`
/// resolve relative to the referring page. Bare names must be unique across
/// the vault, including root pages. Matching is case-sensitive on every OS.
/// A leading `/` is accepted as an explicit vault-root qualifier, but rename
/// output uses conventional folder or source-relative paths instead.
#[derive(Debug, Clone)]
pub struct LinkResolver {
    paths: BTreeSet<PathBuf>,
    qualified: BTreeMap<String, Vec<PathBuf>>,
    basenames: BTreeMap<String, Vec<PathBuf>>,
}

impl LinkResolver {
    pub fn new<I, P>(paths: I) -> Result<Self, LinkError>
    where
        I: IntoIterator<Item = P>,
        P: AsRef<Path>,
    {
        let mut resolver = Self {
            paths: BTreeSet::new(),
            qualified: BTreeMap::new(),
            basenames: BTreeMap::new(),
        };
        for path in paths {
            let path = path.as_ref();
            validate_page_path(path)?;
            if !resolver.paths.insert(path.to_owned()) {
                return Err(LinkError::DuplicatePath(path.to_owned()));
            }
            resolver
                .qualified
                .entry(link_path(path, false))
                .or_default()
                .push(path.to_owned());
            resolver
                .basenames
                .entry(path.file_stem().unwrap().to_str().unwrap().to_owned())
                .or_default()
                .push(path.to_owned());
        }
        for candidates in resolver
            .qualified
            .values_mut()
            .chain(resolver.basenames.values_mut())
        {
            candidates.sort();
        }
        Ok(resolver)
    }

    pub fn resolve(&self, source: &Path, target: &str) -> Result<PathBuf, ResolveError> {
        let invalid = || ResolveError::Invalid(target.to_owned());
        if validate_page_path(source).is_err() {
            return Err(invalid());
        }
        let target = target.trim();
        if target.is_empty() {
            return if self.paths.contains(source) {
                Ok(source.to_owned())
            } else {
                Err(ResolveError::Missing(target.to_owned()))
            };
        }
        if target.contains(['\0', '\r', '\n', ':', '#', '|', '[', ']']) {
            return Err(invalid());
        }
        // Wikilink syntax uses slash-separated components; PathBuf is used for disk paths.
        let target_path = target.replace('\\', "/");
        let relative = target_path.starts_with("./") || target_path.starts_with("../");
        let qualified = relative || target_path.contains('/');
        let mut parts: Vec<String> = if relative {
            source
                .parent()
                .unwrap_or(Path::new(""))
                .components()
                .map(|part| part.as_os_str().to_str().unwrap().to_owned())
                .collect()
        } else {
            Vec::new()
        };
        for (index, part) in target_path.split('/').enumerate() {
            match part {
                "" if index == 0 && target_path.starts_with('/') => {}
                "." if relative => {}
                ".." if relative => {
                    if parts.pop().is_none() {
                        return Err(invalid());
                    }
                }
                "" | "." | ".." | ".selenite" | ".git" => return Err(invalid()),
                part => parts.push(part.to_owned()),
            }
        }
        let Some(last) = parts.last_mut() else {
            return Err(invalid());
        };
        if last.to_ascii_lowercase().ends_with(".md") {
            last.truncate(last.len() - 3);
        }
        if last.is_empty() {
            return Err(invalid());
        }
        let key = parts.join("/");
        let matches = if qualified {
            self.qualified.get(&key)
        } else {
            self.basenames.get(&key)
        };
        match matches.map(Vec::as_slice) {
            Some([path]) => Ok(path.clone()),
            Some(paths) if !paths.is_empty() => Err(ResolveError::Ambiguous {
                target: target.to_owned(),
                candidates: paths.to_vec(),
            }),
            _ => Err(ResolveError::Missing(target.to_owned())),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Backlink {
    pub source: PathBuf,
    pub link: WikiLink,
}

/// Contains resolved links only; missing and ambiguous links are never guessed.
#[derive(Debug, Clone, Default)]
pub struct BacklinkMap {
    entries: BTreeMap<PathBuf, Vec<Backlink>>,
}

impl BacklinkMap {
    pub fn build(pages: &[Page]) -> Result<Self, LinkError> {
        let resolver = LinkResolver::new(pages.iter().map(|page| &page.path))?;
        let mut map = Self::default();
        for page in pages {
            for link in &page.links {
                if let Ok(target) = resolver.resolve(&page.path, &link.target) {
                    map.entries.entry(target).or_default().push(Backlink {
                        source: page.path.clone(),
                        link: link.clone(),
                    });
                }
            }
        }
        for backlinks in map.entries.values_mut() {
            backlinks.sort_by(|left, right| {
                (&left.source, left.link.span.start).cmp(&(&right.source, right.link.span.start))
            });
        }
        Ok(map)
    }

    pub fn backlinks(&self, target: &Path) -> &[Backlink] {
        self.entries
            .get(target)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone)]
pub struct RenamePlan {
    pub from: PathBuf,
    pub to: PathBuf,
    /// Only changed pages, including the renamed page, sorted by resulting path.
    /// No filesystem mutation has occurred; callers must persist these sources.
    pub pages: Vec<Page>,
}

/// Preserve every previously resolved link identity, including outgoing relative
/// links from the moving page and unrelated shorthand links affected by a new
/// basename collision. Unresolved links and local fragments remain unchanged.
pub fn plan_rename(pages: &[Page], from: &Path, to: &Path) -> Result<RenamePlan, LinkError> {
    validate_page_path(from)?;
    validate_page_path(to)?;
    let before = LinkResolver::new(pages.iter().map(|page| &page.path))?;
    if !before.paths.contains(from) {
        return Err(LinkError::MissingSource(from.to_owned()));
    }
    let mut plan = RenamePlan {
        from: from.to_owned(),
        to: to.to_owned(),
        pages: Vec::new(),
    };
    if from == to {
        return Ok(plan);
    }
    // Reject collisions that would overwrite an existing file on Windows too.
    if before.paths.iter().any(|path| {
        path != from && link_path(path, true).eq_ignore_ascii_case(&link_path(to, true))
    }) {
        return Err(LinkError::DestinationExists(to.to_owned()));
    }
    let after = LinkResolver::new(
        pages
            .iter()
            .map(|page| if page.path == from { to } else { &page.path }),
    )?;
    for original in pages {
        // Reparse source so callers cannot accidentally apply stale public spans.
        let page = Page::parse(&original.path, &original.source)?;
        let new_source = if page.path == from { to } else { &page.path };
        let mut replacements = Vec::new();
        for link in &page.links {
            if link.target.is_empty() {
                continue;
            }
            let Ok(previous_target) = before.resolve(&page.path, &link.target) else {
                continue;
            };
            let desired = if previous_target == from {
                to
            } else {
                &previous_target
            };
            if after.resolve(new_source, &link.target).as_deref() == Ok(desired) {
                continue;
            }
            let include_extension = link.target.to_ascii_lowercase().ends_with(".md");
            let short = if include_extension {
                desired.file_name().unwrap()
            } else {
                desired.file_stem().unwrap()
            }
            .to_str()
            .unwrap();
            let replacement = if !link.target.contains(['/', '\\'])
                && after.resolve(new_source, short).as_deref() == Ok(desired)
            {
                short.to_owned()
            } else if link.target.starts_with("./")
                || link.target.starts_with("../")
                || desired
                    .parent()
                    .is_none_or(|parent| parent.as_os_str().is_empty())
            {
                relative_link(new_source, desired, include_extension)
            } else {
                link_path(desired, include_extension)
            };
            if after.resolve(new_source, &replacement).as_deref() != Ok(desired) {
                return Err(LinkError::UnrepresentableTarget(desired.to_owned()));
            }
            replacements.push((link.target_span.clone(), replacement));
        }
        let mut source = page.source.clone();
        for (span, replacement) in replacements.into_iter().rev() {
            source.replace_range(span, &replacement);
        }
        if new_source != page.path || source != page.source {
            plan.pages.push(Page::parse(new_source, &source)?);
        }
    }
    plan.pages.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(plan)
}

fn link_path(path: &Path, include_extension: bool) -> String {
    let mut parts: Vec<_> = path
        .components()
        .map(|component| component.as_os_str().to_str().unwrap().to_owned())
        .collect();
    if !include_extension {
        *parts.last_mut().unwrap() = path.file_stem().unwrap().to_str().unwrap().to_owned();
    }
    parts.join("/")
}

fn relative_link(source: &Path, target: &Path, include_extension: bool) -> String {
    let from: Vec<_> = source
        .parent()
        .unwrap_or(Path::new(""))
        .components()
        .collect();
    let to: Vec<_> = target.components().collect();
    let common = from
        .iter()
        .zip(&to)
        .take_while(|(left, right)| left == right)
        .count();
    let mut parts = vec!["..".to_owned(); from.len() - common];
    parts.extend(
        to[common..]
            .iter()
            .map(|part| part.as_os_str().to_str().unwrap().to_owned()),
    );
    if !include_extension {
        *parts.last_mut().unwrap() = target.file_stem().unwrap().to_str().unwrap().to_owned();
    }
    if from.len() == common {
        parts.insert(0, ".".to_owned());
    }
    parts.join("/")
}
