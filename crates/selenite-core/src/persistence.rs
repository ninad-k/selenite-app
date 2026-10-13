//! Conflict-aware local persistence. Snapshots and interrupted transactions are
//! retained; no cleanup or garbage collection silently deletes user history.

use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use serde_yaml::{Mapping, Value};

use crate::{
    Page, plan_rename, validate_page_path,
    vault::{Vault, validate_relative},
};

#[derive(Debug, Clone)]
pub struct WriteReport {
    pub path: PathBuf,
    pub revision: String,
    /// Vault-relative path of the retained pre-write Markdown snapshot.
    pub history: PathBuf,
}

#[derive(Debug, Clone)]
pub struct RenameReport {
    pub from: PathBuf,
    pub to: PathBuf,
    pub revision: String,
    pub updated: Vec<PathBuf>,
    /// Vault-relative directory containing the journal and source snapshots.
    pub journal: PathBuf,
}

impl Vault {
    /// Guard a multi-file read against cooperating Selenite writers. Individual
    /// reads remain lock-free and see complete files after atomic publication.
    pub fn lock_shared(&self) -> io::Result<File> {
        let file = write_lock_file(self)?;
        file.lock_shared()?;
        Ok(file)
    }

    /// Compare exact source bytes, stage and sync, then atomically replace the
    /// page. Validation, staging and publication errors preserve the old page.
    /// Arbitrary external editors do not participate in the advisory lock.
    pub fn write_page(
        &self,
        path: impl AsRef<Path>,
        expected_source: &str,
        new_source: &str,
    ) -> io::Result<WriteReport> {
        let path = path.as_ref();
        validate_markdown(path, new_source)?;
        let _guard = exclusive(self)?;
        let destination = destination(self.root(), path, false)?;
        compare(&destination, Some(expected_source))?;
        let (revision, directory) = revision_directory(self, "history")?;
        write_new(&directory.join("page.txt"), &key(path))?;
        write_new(&directory.join("before.md"), expected_source)?;
        write_new(&directory.join("after.md"), new_source)?;
        let staged = directory.join("publish.md");
        write_new(&staged, new_source)?;
        // Recheck after slow staging, narrowing races with non-cooperating editors.
        compare(&destination, Some(expected_source))?;
        fs::rename(&staged, &destination)?;
        Ok(WriteReport {
            path: path.to_owned(),
            revision,
            history: directory
                .join("before.md")
                .strip_prefix(self.root())
                .unwrap()
                .to_owned(),
        })
    }

    /// Metadata paths are relative to `.selenite`, e.g. `schemas/tasks.toml`.
    pub fn read_metadata(&self, path: impl AsRef<Path>) -> io::Result<Option<String>> {
        let path = path.as_ref();
        metadata_path(path)?;
        let root = metadata_root(self, false)?;
        let destination = match destination(&root, path, false) {
            Ok(path) => path,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        read_optional(&destination)
    }

    /// Enforce a size limit before allocating an untrusted metadata document.
    pub fn read_metadata_bounded(
        &self,
        path: impl AsRef<Path>,
        max_bytes: usize,
    ) -> io::Result<Option<String>> {
        let path = path.as_ref();
        metadata_path(path)?;
        let root = metadata_root(self, false)?;
        let destination = match destination(&root, path, false) {
            Ok(path) => path,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        regular_file_or_absent(&destination)?;
        match crate::vault::read_bounded(&destination, max_bytes) {
            Ok(value) => Ok(Some(value)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// None requires absence; Some requires exact prior bytes. A conflict never
    /// overwrites either value. The original value and staged bytes are retained.
    pub fn write_metadata(
        &self,
        path: impl AsRef<Path>,
        expected: Option<&str>,
        content: &str,
    ) -> io::Result<()> {
        let path = path.as_ref();
        metadata_path(path)?;
        let _guard = exclusive(self)?;
        let root = metadata_root(self, true)?;
        let destination = destination(&root, path, true)?;
        compare(&destination, expected)?;
        if expected == Some(content) {
            return Ok(());
        }
        let (_, directory) = revision_directory(self, "history")?;
        write_new(&directory.join("metadata.txt"), &key(path))?;
        if let Some(previous) = expected {
            write_new(&directory.join("before.txt"), previous)?;
        }
        write_new(&directory.join("after.txt"), content)?;
        let staged = directory.join("publish.txt");
        write_new(&staged, content)?;
        compare(&destination, expected)?;
        if expected.is_some() {
            fs::rename(staged, destination)?;
        } else {
            fs::hard_link(staged, destination)?;
        }
        Ok(())
    }

    pub fn metadata_paths(&self, prefix: impl AsRef<Path>) -> io::Result<Vec<PathBuf>> {
        let prefix = prefix.as_ref();
        metadata_path(prefix)?;
        let root = metadata_root(self, false)?;
        let first = match destination(&root, prefix, false) {
            Ok(path) => path,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };
        if !first.exists() {
            return Ok(Vec::new());
        }
        let mut pending = vec![first];
        let mut files = Vec::new();
        while let Some(path) = pending.pop() {
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.is_dir() {
                for entry in fs::read_dir(&path)? {
                    pending.push(entry?.path());
                }
            } else if metadata.is_file() {
                let relative = path.strip_prefix(&root).map_err(invalid)?.to_owned();
                metadata_path(&relative)?;
                files.push(relative);
            }
        }
        files.sort();
        Ok(files)
    }

    /// Apply a previously reviewed `plan_rename(expected_pages, from, to)`.
    /// The entire vault file set and every source are preconditions: stale plans
    /// fail before publishing any page. Multi-file publication is journaled,
    /// rather than pretending that a filesystem-wide atomic rename exists.
    pub fn apply_rename(
        &self,
        from: &Path,
        to: &Path,
        expected_pages: &[Page],
    ) -> io::Result<RenameReport> {
        self.apply_rename_with_edits(from, to, expected_pages, &[])
    }

    /// Apply additional frontmatter edits in the same rename journal. Overlays
    /// must name unique existing post-rename pages and preserve every body byte
    /// produced by the core link planner. Domain schemas remain outside core.
    pub fn apply_rename_with_edits(
        &self,
        from: &Path,
        to: &Path,
        expected_pages: &[Page],
        final_additional_pages: &[Page],
    ) -> io::Result<RenameReport> {
        if from == to {
            return Err(invalid("rename source and destination are identical"));
        }
        let _guard = exclusive(self)?;
        let journal = prepare_rename(self, from, to, expected_pages, final_additional_pages)?;
        complete_rename(self, &journal).map_err(|error| {
            io::Error::new(
                error.kind(),
                format!(
                    "{error}; retained rename recovery journal: {}",
                    journal.directory.display()
                ),
            )
        })
    }

    /// Resume interrupted prepared transactions. Current files must still match
    /// their recorded before/after bytes; external conflicts stop recovery.
    pub fn recover_pending_renames(&self) -> io::Result<Vec<RenameReport>> {
        let _guard = exclusive(self)?;
        let root = metadata_root(self, true)?;
        let transactions = destination(&root, Path::new("transactions"), false)?;
        if !transactions.exists() {
            return Ok(Vec::new());
        }
        regular_directory(&transactions)?;
        let mut paths = fs::read_dir(&transactions)?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<io::Result<Vec<_>>>()?;
        paths.sort();
        let mut recovered = Vec::new();
        for directory in paths {
            regular_directory(&directory)?;
            if let Some(marker) = read_optional(&directory.join("committed"))? {
                if marker == "committed\n" {
                    continue;
                }
                return Err(invalid("invalid rename completion marker"));
            }
            if let Some(manifest) = read_optional(&directory.join("rename.yaml"))? {
                let journal = Journal::read(directory, &manifest)?;
                recovered.push(complete_rename(self, &journal)?);
            }
        }
        Ok(recovered)
    }
}

pub(crate) fn create_page(vault: &Vault, path: &Path, source: &str) -> io::Result<bool> {
    validate_page_path(path).map_err(invalid)?;
    let _guard = exclusive(vault)?;
    let destination = destination(vault.root(), path, true)?;
    if read_optional(&destination)?.is_some() {
        return Ok(false);
    }
    validate_markdown(path, source)?;
    let (_, directory) = revision_directory(vault, "history")?;
    write_new(&directory.join("page.txt"), &key(path))?;
    write_new(&directory.join("after.md"), source)?;
    let staged = directory.join("publish.md");
    write_new(&staged, source)?;
    // Atomic exclusive creation: link a fully synced file, never expose a
    // partially written note. Retain staging because cleanup is not authorized.
    match fs::hard_link(staged, &destination) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            read_optional(&destination)?;
            Ok(false)
        }
        Err(error) => Err(error),
    }
}

struct Edit {
    source: PathBuf,
    target: PathBuf,
    before: String,
    after: String,
}
struct Journal {
    directory: PathBuf,
    from: PathBuf,
    to: PathBuf,
    edits: Vec<Edit>,
    unchanged: BTreeMap<PathBuf, String>,
}

fn prepare_rename(
    vault: &Vault,
    from: &Path,
    to: &Path,
    expected: &[Page],
    additional: &[Page],
) -> io::Result<Journal> {
    let mut sources = BTreeMap::new();
    let mut parsed = Vec::new();
    for page in expected {
        parsed.push(Page::parse(&page.path, &page.source).map_err(invalid)?);
        if sources.insert(page.path.clone(), &page.source).is_some() {
            return Err(invalid("duplicate expected page"));
        }
    }
    let actual = vault.markdown_paths()?;
    if actual != sources.keys().cloned().collect::<Vec<_>>() {
        return Err(conflict("vault page set changed since rename planning"));
    }
    for path in &actual {
        compare(
            &vault.resolve_existing(path)?,
            sources.get(path).map(|source| source.as_str()),
        )?;
    }
    let plan = plan_rename(&parsed, from, to).map_err(invalid)?;
    let mut final_pages: BTreeMap<_, _> = plan
        .pages
        .into_iter()
        .map(|page| (page.path.clone(), page))
        .collect();
    let mut seen = std::collections::BTreeSet::new();
    for overlay in additional {
        let overlay = Page::parse(&overlay.path, &overlay.source).map_err(invalid)?;
        if !seen.insert(overlay.path.clone()) {
            return Err(invalid("duplicate rename overlay path"));
        }
        let base = final_pages
            .get(&overlay.path)
            .or_else(|| {
                parsed
                    .iter()
                    .find(|page| page.path != from && page.path == overlay.path)
            })
            .ok_or_else(|| invalid("rename overlay does not name an existing post-rename page"))?;
        if overlay.body != base.body {
            return Err(invalid(
                "rename overlay changes core-planned Markdown body bytes",
            ));
        }
        final_pages.insert(overlay.path.clone(), overlay);
    }
    final_pages.retain(|path, page| {
        path == to
            || sources
                .get(path)
                .is_none_or(|source| *source != &page.source)
    });
    // Resolve all parent paths and verify destinations before source mutations.
    for page in final_pages.values() {
        destination(vault.root(), &page.path, true)?;
    }
    let (_, directory) = revision_directory(vault, "transactions")?;
    let mut edits = Vec::new();
    for (number, page) in final_pages.into_values().enumerate() {
        let source = if page.path == to {
            from.to_owned()
        } else {
            page.path.clone()
        };
        let before = (*sources
            .get(&source)
            .ok_or_else(|| invalid("missing rename source snapshot"))?)
        .clone();
        write_new(&directory.join(format!("{number}-before.md")), &before)?;
        write_new(&directory.join(format!("{number}-after.md")), &page.source)?;
        edits.push(Edit {
            source,
            target: page.path,
            before,
            after: page.source,
        });
    }
    let mut unchanged = BTreeMap::new();
    for (path, source) in sources {
        if !edits.iter().any(|edit| edit.source == path) {
            write_new(
                &directory.join(format!("unchanged-{}.md", unchanged.len())),
                source,
            )?;
            unchanged.insert(path, source.clone());
        }
    }
    let journal = Journal {
        directory,
        from: from.to_owned(),
        to: to.to_owned(),
        edits,
        unchanged,
    };
    preflight(vault, &journal)?;
    let same_windows_path = cfg!(windows) && key(from).eq_ignore_ascii_case(&key(to));
    if !same_windows_path {
        compare(&destination(vault.root(), to, false)?, None)?;
    }
    let staged = journal.directory.join("prepared.yaml");
    write_new(&staged, &journal.manifest()?)?;
    fs::rename(staged, journal.directory.join("rename.yaml"))?;
    Ok(journal)
}

fn preflight(vault: &Vault, journal: &Journal) -> io::Result<()> {
    for path in vault.markdown_paths()? {
        if !journal.unchanged.contains_key(&path)
            && !journal
                .edits
                .iter()
                .any(|edit| edit.source == path || edit.target == path)
        {
            return Err(conflict(format!(
                "unexpected page during rename recovery: {}",
                path.display()
            )));
        }
    }
    for (path, source) in &journal.unchanged {
        compare(&destination(vault.root(), path, false)?, Some(source))?;
    }
    let archive = read_optional(&journal.directory.join("source-archive.md"))?;
    let moved = journal
        .edits
        .iter()
        .find(|edit| edit.source == journal.from && edit.target == journal.to)
        .ok_or_else(|| invalid("rename journal has no moved page"))?;
    let from = destination(vault.root(), &journal.from, false)?;
    let same_windows_path =
        cfg!(windows) && key(&journal.from).eq_ignore_ascii_case(&key(&journal.to));
    if let Some(archive) = archive {
        if archive != moved.before {
            return Err(conflict("retained rename source was modified"));
        }
        if !same_windows_path && read_optional(&from)?.is_some() {
            return Err(conflict("rename source was recreated after archival"));
        }
    } else {
        compare(&from, Some(&moved.before))?;
    }
    for edit in &journal.edits {
        validate_markdown(&edit.source, &edit.before)?;
        validate_markdown(&edit.target, &edit.after)?;
        let path = destination(vault.root(), &edit.target, true)?;
        let actual = read_optional(&path)?;
        let allowed = if edit.source == journal.from {
            actual.as_deref().is_none_or(|source| {
                source == edit.after || (same_windows_path && source == edit.before)
            })
        } else {
            actual
                .as_deref()
                .is_some_and(|source| source == edit.before || source == edit.after)
        };
        if !allowed {
            return Err(conflict(format!(
                "rename target changed: {}",
                edit.target.display()
            )));
        }
    }
    Ok(())
}

fn complete_rename(vault: &Vault, journal: &Journal) -> io::Result<RenameReport> {
    preflight(vault, journal)?;
    let archive = journal.directory.join("source-archive.md");
    if read_optional(&archive)?.is_none() {
        let source = vault.resolve_existing(&journal.from)?;
        fs::rename(source, &archive)?;
    }
    for edit in &journal.edits {
        let target = destination(vault.root(), &edit.target, true)?;
        if read_optional(&target)?.as_deref() == Some(&edit.after) {
            continue;
        }
        let staged = journal.directory.join(format!("publish-{}.md", revision()));
        write_new(&staged, &edit.after)?;
        if edit.source == journal.from {
            compare(&target, None)?;
            fs::hard_link(staged, target)?;
        } else {
            compare(&target, Some(&edit.before))?;
            fs::rename(staged, target)?;
        }
    }
    let marker = journal.directory.join(format!("commit-{}", revision()));
    write_new(&marker, "committed\n")?;
    fs::rename(marker, journal.directory.join("committed"))?;
    Ok(RenameReport {
        from: journal.from.clone(),
        to: journal.to.clone(),
        revision: journal
            .directory
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned(),
        updated: journal
            .edits
            .iter()
            .map(|edit| edit.target.clone())
            .collect(),
        journal: journal
            .directory
            .strip_prefix(vault.root())
            .unwrap()
            .to_owned(),
    })
}

impl Journal {
    fn manifest(&self) -> io::Result<String> {
        let mut manifest = Mapping::new();
        manifest.insert("format".into(), 1.into());
        manifest.insert("from".into(), key(&self.from).into());
        manifest.insert("to".into(), key(&self.to).into());
        manifest.insert(
            "edits".into(),
            Value::Sequence(
                self.edits
                    .iter()
                    .map(|edit| {
                        let mut item = Mapping::new();
                        item.insert("source".into(), key(&edit.source).into());
                        item.insert("target".into(), key(&edit.target).into());
                        Value::Mapping(item)
                    })
                    .collect(),
            ),
        );
        manifest.insert(
            "unchanged".into(),
            Value::Sequence(
                self.unchanged
                    .keys()
                    .map(|path| Value::String(key(path)))
                    .collect(),
            ),
        );
        serde_yaml::to_string(&manifest).map_err(invalid)
    }

    fn read(directory: PathBuf, source: &str) -> io::Result<Self> {
        let value: Value = serde_yaml::from_str(source).map_err(invalid)?;
        if value["format"].as_u64() != Some(1) {
            return Err(invalid("unsupported rename journal format"));
        }
        let from = manifest_path(&value["from"])?;
        let to = manifest_path(&value["to"])?;
        if from == to {
            return Err(invalid("identical rename journal paths"));
        }
        let entries = value["edits"]
            .as_sequence()
            .ok_or_else(|| invalid("rename journal edits are missing"))?;
        let mut targets = BTreeMap::new();
        let mut edits = Vec::new();
        for (number, item) in entries.iter().enumerate() {
            let source = manifest_path(&item["source"])?;
            let target = manifest_path(&item["target"])?;
            if source != target && (source != from || target != to) {
                return Err(invalid("unexpected source in rename journal"));
            }
            if targets.insert(target.clone(), ()).is_some() {
                return Err(invalid("duplicate rename target"));
            }
            let before = read_optional(&directory.join(format!("{number}-before.md")))?
                .ok_or_else(|| invalid("rename before snapshot missing"))?;
            let after = read_optional(&directory.join(format!("{number}-after.md")))?
                .ok_or_else(|| invalid("rename after snapshot missing"))?;
            edits.push(Edit {
                source,
                target,
                before,
                after,
            });
        }
        let mut unchanged = BTreeMap::new();
        let entries = value["unchanged"]
            .as_sequence()
            .ok_or_else(|| invalid("rename unchanged snapshots are missing"))?;
        for (number, entry) in entries.iter().enumerate() {
            let path = manifest_path(entry)?;
            if edits
                .iter()
                .any(|edit| edit.source == path || edit.target == path)
            {
                return Err(invalid("overlapping rename snapshot paths"));
            }
            let source = read_optional(&directory.join(format!("unchanged-{number}.md")))?
                .ok_or_else(|| invalid("unchanged rename snapshot missing"))?;
            validate_markdown(&path, &source)?;
            if unchanged.insert(path, source).is_some() {
                return Err(invalid("duplicate unchanged snapshot path"));
            }
        }
        Ok(Self {
            directory,
            from,
            to,
            edits,
            unchanged,
        })
    }
}

fn manifest_path(value: &Value) -> io::Result<PathBuf> {
    let text = value
        .as_str()
        .ok_or_else(|| invalid("invalid journal path"))?;
    validate_page_path(Path::new(text)).map_err(invalid)?;
    Ok(text.split('/').collect())
}

fn validate_markdown(path: &Path, source: &str) -> io::Result<()> {
    Page::parse(path, source).map(|_| ()).map_err(invalid)
}
fn key(path: &Path) -> String {
    path.components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}
fn invalid(error: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, error.to_string())
}
fn conflict(error: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::AlreadyExists, error.to_string())
}

fn metadata_path(path: &Path) -> io::Result<()> {
    validate_relative(path)?;
    let first = path
        .components()
        .next()
        .unwrap()
        .as_os_str()
        .to_string_lossy()
        .to_ascii_lowercase();
    if matches!(first.as_str(), "write.lock" | "history" | "transactions") {
        return Err(invalid("metadata path is reserved for persistence"));
    }
    Ok(())
}

fn metadata_root(vault: &Vault, create: bool) -> io::Result<PathBuf> {
    let root = vault.root().join(".selenite");
    if create {
        ensure_directory(&root)?;
    } else {
        match fs::symlink_metadata(&root) {
            Ok(_) => regular_directory(&root)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(root)
}

fn write_lock_file(vault: &Vault) -> io::Result<File> {
    let path = metadata_root(vault, true)?.join("write.lock");
    regular_file_or_absent(&path)?;
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
}

fn exclusive(vault: &Vault) -> io::Result<File> {
    let file = write_lock_file(vault)?;
    file.lock()?;
    Ok(file)
}

fn ensure_directory(path: &Path) -> io::Result<()> {
    match fs::create_dir(path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    regular_directory(path)
}

fn regular_directory(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(invalid(format!(
            "not a regular directory: {}",
            path.display()
        )));
    }
    Ok(())
}

fn regular_file_or_absent(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => Ok(()),
        Ok(_) => Err(invalid(format!("not a regular file: {}", path.display()))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn destination(root: &Path, relative: &Path, create_parents: bool) -> io::Result<PathBuf> {
    validate_relative(relative)?;
    regular_directory(root)?;
    let mut path = root.to_owned();
    let components: Vec<_> = relative.components().collect();
    for (number, component) in components.iter().enumerate() {
        path.push(component.as_os_str());
        if number + 1 != components.len() {
            if create_parents {
                ensure_directory(&path)?;
            } else {
                regular_directory(&path)?;
            }
        } else {
            match fs::symlink_metadata(&path) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err(invalid("symbolic link destination rejected"));
                }
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
    }
    Ok(path)
}

fn read_optional(path: &Path) -> io::Result<Option<String>> {
    regular_file_or_absent(path)?;
    match fs::read_to_string(path) {
        Ok(source) => Ok(Some(source)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn compare(path: &Path, expected: Option<&str>) -> io::Result<()> {
    let current = read_optional(path)?;
    if current.as_deref() == expected {
        return Ok(());
    }
    if current.is_none() && expected.is_some() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("expected file disappeared: {}", path.display()),
        ));
    }
    Err(conflict(format!("source changed: {}", path.display())))
}

fn write_new(path: &Path, source: &str) -> io::Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(source.as_bytes())?;
    file.sync_all()
}

fn revision() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    format!(
        "{}-{}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

fn revision_directory(vault: &Vault, namespace: &str) -> io::Result<(String, PathBuf)> {
    let parent = metadata_root(vault, true)?.join(namespace);
    ensure_directory(&parent)?;
    loop {
        let revision = revision();
        let path = parent.join(&revision);
        match fs::create_dir(&path) {
            Ok(()) => return Ok((revision, path)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
}

#[cfg(test)]
mod tests;
