//! Disposable local indexes. Every cached value can be recovered from Markdown.

mod incremental;
mod search;
mod watcher;
pub use incremental::RefreshStats;
pub use search::SearchHit;
pub use watcher::VaultWatcher;

use std::{
    collections::BTreeMap,
    fs::{self, File},
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, Transaction, TransactionBehavior, params};
use selenite_core::{LinkResolver, Page, vault::Vault};

const SCHEMA: &str = include_str!("schema.sql");

#[derive(Debug)]
struct IncompatibleCache;

impl std::fmt::Display for IncompatibleCache {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("incompatible cache version; rebuild from disk")
    }
}

impl std::error::Error for IncompatibleCache {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedPage {
    pub path: PathBuf,
    pub title: String,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedLink {
    pub source: PathBuf,
    pub target: String,
    pub resolved: Option<PathBuf>,
    pub heading: Option<String>,
    pub alias: Option<String>,
    pub embedded: bool,
}

#[derive(Debug, Clone)]
pub struct RebuildStats {
    pub pages: usize,
    pub links: usize,
    pub properties: usize,
    pub elapsed: Duration,
}

pub struct Index {
    vault: Vault,
    cache_dir: PathBuf,
    connection: Connection,
    search: search::SearchIndex,
    cache_identity: String,
}

impl Index {
    /// Opens a cache, creating an empty schema if needed. Call `rebuild` to
    /// populate it. This never creates or edits a Markdown page.
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        let vault = Vault::open(root).context("open vault")?;
        let cache_dir = prepare_cache(&vault)?;
        let _guard = cache_lock(&cache_dir, true)?;
        Self::open_unlocked(vault, cache_dir)
    }

    fn open_unlocked(vault: Vault, cache_dir: PathBuf) -> Result<Self> {
        let identity_path = cache_dir.join("identity");
        check_cache_file(&identity_path)?;
        let cache_identity = match fs::read_to_string(&identity_path) {
            Ok(identity) => identity,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let identity = format!(
                    "{}-{}",
                    std::process::id(),
                    SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
                );
                fs::write(&identity_path, &identity)?;
                identity
            }
            Err(error) => return Err(error.into()),
        };
        for name in [
            "index.sqlite",
            "index.sqlite-wal",
            "index.sqlite-shm",
            "index.sqlite-journal",
        ] {
            check_cache_file(&cache_dir.join(name))?;
        }
        let connection = Connection::open(cache_dir.join("index.sqlite"))?;
        connection.busy_timeout(Duration::from_secs(5))?;
        let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if !matches!(version, 0 | 1) {
            return Err(IncompatibleCache.into());
        }
        connection.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")?;
        connection.execute_batch(SCHEMA)?;
        // Validate columns as well as the version before exposing cache reads.
        connection.prepare("SELECT path, title, body, source FROM pages LIMIT 0")?;
        connection.prepare("SELECT source, position, target, resolved, heading, alias, embedded FROM links LIMIT 0")?;
        connection.prepare("SELECT page, name, value_yaml FROM properties LIMIT 0")?;
        connection.prepare("SELECT ready FROM cache_state WHERE singleton = 1")?;
        let search = search::SearchIndex::open(&cache_dir.join("search"))?;
        if search.is_new() {
            connection.execute("UPDATE cache_state SET ready = 0", [])?;
        }
        Ok(Self {
            vault,
            cache_dir,
            connection,
            search,
            cache_identity,
        })
    }

    pub fn vault(&self) -> &Vault {
        &self.vault
    }

    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    pub fn is_ready(&self) -> Result<bool> {
        let _guard = cache_lock(&self.cache_dir, false)?;
        self.ready_unlocked()
    }

    fn ready_unlocked(&self) -> Result<bool> {
        ensure!(
            fs::read_to_string(self.cache_dir.join("identity"))? == self.cache_identity,
            "cache was replaced; reopen the index"
        );
        Ok(self.connection.query_row(
            "SELECT ready FROM cache_state WHERE singleton = 1",
            [],
            |row| row.get(0),
        )?)
    }

    fn ensure_ready(&self) -> Result<()> {
        ensure!(
            self.ready_unlocked()?,
            "cache is incomplete; rebuild from disk before querying"
        );
        Ok(())
    }

    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchHit>> {
        let _guard = cache_lock(&self.cache_dir, false)?;
        self.ensure_ready()?;
        self.search.reload()?;
        self.search.search(query, limit)
    }

    /// Parse all sources before touching the previous cache. Recreate every
    /// table and row within one SQLite transaction; no cached content is input.
    pub fn rebuild(&mut self) -> Result<RebuildStats> {
        let started = Instant::now();
        let pages = self.read_pages()?;
        self.rebuild_pages(pages, started)
    }

    /// Recovery entry point that works even when the existing cache is corrupt.
    /// Invalid sources abort before any cache is moved. Damaged cache files are
    /// retained together in a sibling directory for inspection, never used as input.
    pub fn rebuild_from_disk(root: impl AsRef<Path>) -> Result<(Self, RebuildStats)> {
        let started = Instant::now();
        let vault = Vault::open(root.as_ref())?;
        let pages = read_vault_pages(&vault)?;
        let cache = prepare_cache(&vault)?;
        let _guard = cache_lock(&cache, true)?;
        let mut index = match Self::open_unlocked(vault.clone(), cache.clone()) {
            Ok(index) => index,
            Err(error) if recoverable_cache_error(&error) => {
                let cache = prepare_cache(&vault)?;
                let archive = cache.with_file_name(format!(
                    "cache-retired-{}",
                    SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
                ));
                ensure!(
                    cache.canonicalize()?.starts_with(vault.root()),
                    "cache escapes vault"
                );
                ensure!(
                    archive
                        .parent()
                        .context("cache parent")?
                        .canonicalize()?
                        .starts_with(vault.root()),
                    "archive escapes vault"
                );
                ensure!(!archive.exists(), "cache archive already exists");
                fs::rename(&cache, &archive).context("retain damaged cache")?;
                Self::open_unlocked(vault.clone(), prepare_cache(&vault)?)?
            }
            Err(error) => return Err(error),
        };
        let stats = index.rebuild_pages_unlocked(pages, started)?;
        Ok((index, stats))
    }

    fn rebuild_pages(&mut self, pages: Vec<Page>, started: Instant) -> Result<RebuildStats> {
        let _guard = cache_lock(&self.cache_dir, true)?;
        self.ready_unlocked()?;
        self.rebuild_pages_unlocked(pages, started)
    }

    fn rebuild_pages_unlocked(
        &mut self,
        pages: Vec<Page>,
        started: Instant,
    ) -> Result<RebuildStats> {
        let resolver = LinkResolver::new(pages.iter().map(|page| &page.path))?;
        // Durable before either cache changes. A crash between the two commits
        // leaves this flag false, forcing disk-only recovery before queries.
        self.connection
            .execute("UPDATE cache_state SET ready = 0", [])?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute_batch("DROP TABLE IF EXISTS links; DROP TABLE IF EXISTS properties; DROP TABLE IF EXISTS pages; DROP TABLE IF EXISTS cache_state;")?;
        transaction.execute_batch(SCHEMA)?;
        for page in &pages {
            insert_page(&transaction, page, &resolver)
                .with_context(|| format!("index {}", page.path.display()))?;
        }
        self.search.replace_all(&pages)?;
        transaction.execute("UPDATE cache_state SET ready = 1", [])?;
        transaction.pragma_update(None, "user_version", 1)?;
        transaction.commit()?;
        Ok(RebuildStats {
            pages: pages.len(),
            links: pages.iter().map(|page| page.links.len()).sum(),
            properties: pages.iter().map(|page| page.frontmatter.len()).sum(),
            elapsed: started.elapsed(),
        })
    }

    pub fn pages(&self) -> Result<Vec<IndexedPage>> {
        let _guard = cache_lock(&self.cache_dir, false)?;
        self.ensure_ready()?;
        let mut statement = self
            .connection
            .prepare("SELECT path, title, body FROM pages ORDER BY path")?;
        let rows = statement.query_map([], |row| {
            Ok(IndexedPage {
                path: path_from_key(&row.get::<_, String>(0)?),
                title: row.get(1)?,
                body: row.get(2)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn backlinks(&self, target: &Path) -> Result<Vec<IndexedLink>> {
        let _guard = cache_lock(&self.cache_dir, false)?;
        self.ensure_ready()?;
        selenite_core::validate_page_path(target)?;
        let mut statement = self.connection.prepare("SELECT source, target, resolved, heading, alias, embedded FROM links WHERE resolved = ?1 ORDER BY source, position")?;
        let rows = statement.query_map([path_key(target)], |row| {
            Ok(IndexedLink {
                source: path_from_key(&row.get::<_, String>(0)?),
                target: row.get(1)?,
                resolved: row
                    .get::<_, Option<String>>(2)?
                    .as_deref()
                    .map(path_from_key),
                heading: row.get(3)?,
                alias: row.get(4)?,
                embedded: row.get(5)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn properties(&self, path: &Path) -> Result<BTreeMap<String, String>> {
        let _guard = cache_lock(&self.cache_dir, false)?;
        self.ensure_ready()?;
        selenite_core::validate_page_path(path)?;
        let mut statement = self
            .connection
            .prepare("SELECT name, value_yaml FROM properties WHERE page = ?1 ORDER BY name")?;
        let rows = statement.query_map([path_key(path)], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    fn read_pages(&self) -> Result<Vec<Page>> {
        read_vault_pages(&self.vault)
    }
}

fn read_vault_pages(vault: &Vault) -> Result<Vec<Page>> {
    vault
        .markdown_paths()?
        .into_iter()
        .map(|path| {
            let source = vault
                .read_source(&path)
                .with_context(|| format!("read {}", path.display()))?;
            parse_page(&path, &source)
        })
        .collect()
}

fn parse_page(path: &Path, source: &str) -> Result<Page> {
    let page = Page::parse(path, source).with_context(|| format!("parse {}", path.display()))?;
    ensure!(
        page.frontmatter.keys().all(|name| name.as_str().is_some()),
        "frontmatter property names must be strings in {}",
        path.display()
    );
    Ok(page)
}

fn recoverable_cache_error(error: &anyhow::Error) -> bool {
    error.is::<IncompatibleCache>()
        || matches!(
            error.downcast_ref::<rusqlite::Error>(),
            Some(rusqlite::Error::SqlInputError { .. })
        )
        || error
            .downcast_ref::<rusqlite::Error>()
            .and_then(rusqlite::Error::sqlite_error_code)
            .is_some_and(|code| {
                matches!(
                    code,
                    rusqlite::ErrorCode::DatabaseCorrupt
                        | rusqlite::ErrorCode::NotADatabase
                        | rusqlite::ErrorCode::Unknown
                )
            })
}

fn insert_page(transaction: &Transaction<'_>, page: &Page, resolver: &LinkResolver) -> Result<()> {
    let key = path_key(&page.path);
    transaction.execute(
        "INSERT INTO pages(path,title,body,source) VALUES (?1,?2,?3,?4)",
        params![key, page.title, page.body, page.source],
    )?;
    for link in &page.links {
        let resolved = resolver
            .resolve(&page.path, &link.target)
            .ok()
            .map(|path| path_key(&path));
        transaction.execute("INSERT INTO links(source,position,target,resolved,heading,alias,embedded) VALUES (?1,?2,?3,?4,?5,?6,?7)", params![key, link.span.start as i64, link.target, resolved, link.heading, link.alias, link.embedded])?;
    }
    for (name, value) in &page.frontmatter {
        let name = name
            .as_str()
            .context("frontmatter property names must be strings")?;
        transaction.execute(
            "INSERT INTO properties(page,name,value_yaml) VALUES (?1,?2,?3)",
            params![key, name, serde_yaml::to_string(value)?],
        )?;
    }
    Ok(())
}

/// Forward slashes are an index serialization format, never disk path joining.
pub fn path_key(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

pub fn path_from_key(key: &str) -> PathBuf {
    key.split('/').collect()
}

fn prepare_cache(vault: &Vault) -> Result<PathBuf> {
    let mut path = vault.root().to_owned();
    for name in [".selenite", "cache"] {
        path.push(name);
        match fs::symlink_metadata(&path) {
            Ok(metadata) => ensure!(
                metadata.is_dir() && !metadata.file_type().is_symlink(),
                "cache directory is not a regular directory: {}",
                path.display()
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => fs::create_dir(&path)?,
            Err(error) => return Err(error.into()),
        }
        ensure!(
            path.canonicalize()?.starts_with(vault.root()),
            "cache path escapes vault"
        );
    }
    Ok(path)
}

fn check_cache_file(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => ensure!(
            metadata.is_file() && !metadata.file_type().is_symlink(),
            "unsafe cache file: {}",
            path.display()
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

fn cache_lock(cache_dir: &Path, exclusive: bool) -> Result<File> {
    let path = cache_dir
        .parent()
        .context("cache parent")?
        .join("index.lock");
    check_cache_file(&path)?;
    let lock = File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    if exclusive {
        lock.lock()?;
    } else {
        lock.lock_shared()?;
    }
    Ok(lock)
}
