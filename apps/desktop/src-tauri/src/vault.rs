//! Desktop vault presentation. Calendar behavior stays outside the core engine.

use std::{fs, io, path::Path};

use anyhow::{Context, Result, ensure};
use chrono::NaiveDate;
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd, html};
use selenite_core::{LinkResolver, Page, vault::Vault};
use selenite_index::path_key;
use serde::Serialize;

const DEFAULT_TEMPLATE: &str = include_str!("../../../../templates/daily.md");

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewPage {
    pub path: String,
    pub title: String,
    pub source: String,
    pub html: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenedVault {
    pub root: String,
    pub name: String,
    pub files: Vec<String>,
    pub today: String,
    pub page: PreviewPage,
    pub created: bool,
}

#[derive(Clone)]
pub struct VaultSession {
    vault: Vault,
}

impl VaultSession {
    pub fn open(root: &Path, date: &str) -> Result<(Self, OpenedVault)> {
        let session = Self {
            vault: Vault::open(root).context("open vault folder")?,
        };
        let opened = session.today(date)?;
        Ok((session, opened))
    }

    pub fn root(&self) -> &Path {
        self.vault.root()
    }

    pub fn vault(&self) -> &Vault {
        &self.vault
    }

    pub fn preview(&self, path: &str, source: &str) -> Result<PreviewPage> {
        let page = Page::parse(path, source)?;
        let html = self.render_page(&page)?;
        Ok(PreviewPage {
            path: path_key(&page.path),
            title: page.title,
            source: source.to_owned(),
            html,
        })
    }

    pub fn read_page(&self, path: &str) -> Result<PreviewPage> {
        let relative = Path::new(path);
        selenite_core::validate_page_path(relative)?;
        let source = self
            .vault
            .read_source(relative)
            .with_context(|| format!("read {path}"))?;
        let page = Page::parse(relative, &source).with_context(|| format!("parse {path}"))?;
        let html = self.render_page(&page)?;
        Ok(PreviewPage {
            path: path_key(&page.path),
            title: page.title,
            html,
            source,
        })
    }

    fn render_page(&self, page: &Page) -> Result<String> {
        if page.links.is_empty() {
            return Ok(render(&page.body));
        }
        let mut paths = self.vault.markdown_paths()?;
        if !paths.contains(&page.path) {
            paths.push(page.path.clone());
        }
        let resolver = LinkResolver::new(paths)?;
        let offset = page.source.len() - page.body.len();
        let mut body = page.body.clone();
        let mut prefix = "SELENITEWIKITOKEN".to_owned();
        while page.source.contains(&prefix) {
            prefix.push('Z');
        }
        let mut replacements = Vec::new();
        for (number, link) in page.links.iter().enumerate().rev() {
            let Ok(target) = resolver.resolve(&page.path, &link.target) else {
                continue;
            };
            let token = format!("{prefix}{number}END");
            let label = link.alias.as_deref().unwrap_or(if link.target.is_empty() {
                &page.title
            } else {
                &link.target
            });
            let href = format!(
                "#page={}&fragment={}",
                percent(&path_key(&target)),
                percent(link.heading.as_deref().unwrap_or_default())
            );
            let anchor = format!("<a href=\"{href}\">{}</a>", escape(label));
            let mut embedded = None;
            if link.embedded {
                let source = if target == page.path {
                    Some(page.source.clone())
                } else {
                    self.vault.read_source_bounded(&target, 1024 * 1024).ok()
                };
                // An unavailable or malformed embedded note must not prevent opening
                // or saving the containing note. Its navigation link remains usable.
                if let Some(target_page) =
                    source.and_then(|source| Page::parse(&target, &source).ok())
                {
                    let excerpt = match link.heading.as_deref() {
                        Some(fragment) if fragment.starts_with('^') => target_page
                            .blocks
                            .iter()
                            .find(|block| block.id.as_deref() == Some(&fragment[1..]))
                            .map(|block| block.text.clone()),
                        None | Some("") => Some(target_page.body),
                        Some(heading) => heading_section(&target_page.body, heading),
                    };
                    if let Some(excerpt) = excerpt {
                        let clipped: String = excerpt.chars().take(20_000).collect();
                        let truncated = if excerpt.chars().count() > 20_000 {
                            "<p>Open the linked note to read the rest.</p>"
                        } else {
                            ""
                        };
                        embedded = Some(format!(
                            "<section class=\"embedded-note\"><header>{anchor}</header>{}{truncated}</section>",
                            render(&clipped)
                        ));
                    }
                }
            }
            body.replace_range(link.span.start - offset..link.span.end - offset, &token);
            replacements.push((token, anchor, embedded));
        }
        let mut html = render(&body);
        for (token, anchor, embedded) in replacements {
            if let Some(embedded) = embedded {
                html = html.replace(&format!("<p>{token}</p>"), &embedded);
            }
            html = html.replace(&token, &anchor);
        }
        Ok(html)
    }

    pub fn today(&self, date: &str) -> Result<OpenedVault> {
        let parsed = NaiveDate::parse_from_str(date, "%Y-%m-%d")
            .context("date must be a real calendar date in YYYY-MM-DD format")?;
        ensure!(
            date.len() == 10 && parsed.format("%Y-%m-%d").to_string() == date,
            "date must use YYYY-MM-DD format"
        );
        let path = Path::new("Journal").join(format!("{date}.md"));
        let created = match self.vault.resolve_existing(&path) {
            Ok(_) => false,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let template = self
                    .template()?
                    .unwrap_or_else(|| DEFAULT_TEMPLATE.to_owned());
                let source = template
                    .replace("{{date}}", date)
                    .replace("{{title}}", date);
                Page::parse(&path, &source)
                    .context("daily note template is not valid Markdown with YAML frontmatter")?;
                self.vault
                    .create_page(&path, &source)
                    .context("create daily note")?
            }
            Err(error) => return Err(error).context("resolve daily note"),
        };
        let today = path_key(&path);
        let mut opened = self.snapshot(&today, date)?;
        opened.created = created;
        Ok(opened)
    }

    /// Refresh presentation around an existing page without creating today's note.
    pub fn snapshot(&self, path: &str, date: &str) -> Result<OpenedVault> {
        let today = path_key(&Path::new("Journal").join(format!("{date}.md")));
        let page = self.read_page(path)?;
        let files = self
            .vault
            .markdown_paths()?
            .iter()
            .map(|path| path_key(path))
            .collect();
        let root = self
            .root()
            .to_str()
            .context("vault path must be valid Unicode")?
            .to_owned();
        let name = self
            .root()
            .file_name()
            .unwrap_or(self.root().as_os_str())
            .to_string_lossy()
            .into_owned();
        Ok(OpenedVault {
            root,
            name,
            files,
            today,
            page,
            created: false,
        })
    }

    fn template(&self) -> Result<Option<String>> {
        let mut path = self.root().to_owned();
        for component in [".selenite", "templates", "daily.md"] {
            path.push(component);
            let metadata = match fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
                Err(error) => return Err(error).context("inspect daily template"),
            };
            ensure!(
                !metadata.file_type().is_symlink(),
                "daily template path contains a symbolic link"
            );
            ensure!(
                if component == "daily.md" {
                    metadata.is_file()
                } else {
                    metadata.is_dir()
                },
                "daily template path is not a regular file or directory"
            );
            ensure!(
                path.canonicalize()?.starts_with(self.root()),
                "daily template escapes the vault"
            );
        }
        Ok(Some(
            fs::read_to_string(path).context("read daily template")?,
        ))
    }
}

fn render(body: &str) -> String {
    let options =
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let mut links = Vec::new();
    let events = Parser::new_ext(body, options).filter_map(|event| match event {
        Event::Html(text) | Event::InlineHtml(text) => Some(Event::Text(text)),
        // Keep alt text as ordinary text; never emit an image element or load it.
        Event::Start(Tag::Image { .. }) | Event::End(TagEnd::Image) => None,
        Event::Start(Tag::Link { ref dest_url, .. }) => {
            let allowed = safe_link(dest_url);
            links.push(allowed);
            allowed.then_some(event)
        }
        Event::End(TagEnd::Link) => links.pop().unwrap_or(false).then_some(event),
        event => Some(event),
    });
    let mut rendered = String::new();
    html::push_html(&mut rendered, events);
    rendered
}

fn safe_link(destination: &str) -> bool {
    if destination.chars().any(char::is_control) {
        return false;
    }
    let destination = destination.trim().to_ascii_lowercase();
    destination.starts_with("https://")
        || destination.starts_with("http://")
        || destination.starts_with("mailto:")
        || (!destination.contains(':') && !destination.starts_with("//"))
}

fn percent(value: &str) -> String {
    let mut output = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            output.push(char::from(byte));
        } else {
            output.push_str(&format!("%{byte:02X}"));
        }
    }
    output
}

fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn heading_section(body: &str, heading: &str) -> Option<String> {
    let mut selected = Vec::new();
    let mut depth = None;
    for line in body.lines() {
        let trimmed = line.trim_start();
        let level = trimmed
            .chars()
            .take_while(|character| *character == '#')
            .count();
        let is_heading =
            (1..=6).contains(&level) && trimmed.chars().nth(level).is_some_and(char::is_whitespace);
        if let Some(depth) = depth {
            if is_heading && level <= depth {
                break;
            }
            selected.push(line);
        } else if is_heading && trimmed[level..].trim().trim_end_matches('#').trim() == heading {
            depth = Some(level);
            selected.push(line);
        }
    }
    depth.map(|_| selected.join("\n"))
}
