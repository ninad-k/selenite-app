//! Desktop vault presentation. Calendar behavior stays outside the core engine.

use std::{fs, io, path::Path};

use anyhow::{Context, Result, ensure};
use chrono::NaiveDate;
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd, html};
use selenite_core::{Page, vault::Vault};
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

    pub fn read_page(&self, path: &str) -> Result<PreviewPage> {
        let relative = Path::new(path);
        selenite_core::validate_page_path(relative)?;
        let source = self
            .vault
            .read_source(relative)
            .with_context(|| format!("read {path}"))?;
        let page = Page::parse(relative, &source).with_context(|| format!("parse {path}"))?;
        Ok(PreviewPage {
            path: path_key(&page.path),
            title: page.title,
            html: render(&page.body),
            source,
        })
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
        let page = self.read_page(&today)?;
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
            created,
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
