//! Reconcile filesystem snapshots while parsing and writing only changed pages.

use std::{
    collections::BTreeMap,
    path::PathBuf,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use rusqlite::{TransactionBehavior, params};
use selenite_core::LinkResolver;

use super::{Index, cache_lock, insert_page, parse_page, path_from_key, path_key};

#[derive(Debug, Clone)]
pub struct RefreshStats {
    pub upserted: usize,
    pub removed: usize,
    pub unchanged: usize,
    pub elapsed: Duration,
}

impl Index {
    /// Compare authoritative bytes, without trusting timestamps or event order.
    /// Enumeration catches missed events and directory moves. Only changed pages
    /// enter the Markdown parser and the two index writers.
    pub fn refresh(&mut self) -> Result<RefreshStats> {
        let started = Instant::now();
        let _guard = cache_lock(&self.cache_dir, true)?;
        if !self.ready_unlocked()? {
            let stats = self.rebuild_pages_unlocked(self.read_pages()?, started)?;
            return Ok(RefreshStats {
                upserted: stats.pages,
                removed: 0,
                unchanged: 0,
                elapsed: stats.elapsed,
            });
        }
        let mut previous: BTreeMap<PathBuf, String> = {
            let mut statement = self.connection.prepare("SELECT path, source FROM pages")?;
            statement
                .query_map([], |row| {
                    Ok((path_from_key(&row.get::<_, String>(0)?), row.get(1)?))
                })?
                .collect::<rusqlite::Result<_>>()?
        };
        // Hold one coherent source snapshot across cooperating multi-file renames.
        let source_guard = self.vault.lock_shared()?;
        let paths = self.vault.markdown_paths()?;
        let resolver = LinkResolver::new(&paths)?;
        let mut changed = Vec::new();
        let mut unchanged = 0;
        for path in &paths {
            let source = self
                .vault
                .read_source(path)
                .with_context(|| format!("read {}", path.display()))?;
            if previous.remove(path).as_deref() == Some(source.as_str()) {
                unchanged += 1;
            } else {
                changed.push(parse_page(path, &source)?);
            }
        }
        drop(source_guard);
        let removed: Vec<_> = previous.into_keys().collect();
        if !changed.is_empty() || !removed.is_empty() {
            self.connection
                .execute("UPDATE cache_state SET ready = 0", [])?;
            let transaction = self
                .connection
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            for path in removed.iter().chain(changed.iter().map(|page| &page.path)) {
                transaction.execute("DELETE FROM pages WHERE path = ?1", [path_key(path)])?;
            }
            for page in &changed {
                insert_page(&transaction, page, &resolver)
                    .with_context(|| format!("index {}", page.path.display()))?;
            }
            // New/deleted basenames can change links from otherwise unchanged pages.
            let links = {
                let mut statement =
                    transaction.prepare("SELECT source, position, target, resolved FROM links")?;
                statement
                    .query_map([], |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, Option<String>>(3)?,
                        ))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?
            };
            for (source, position, target, old_resolution) in links {
                let resolved = resolver
                    .resolve(&path_from_key(&source), &target)
                    .ok()
                    .map(|path| path_key(&path));
                if resolved != old_resolution {
                    transaction.execute(
                        "UPDATE links SET resolved = ?1 WHERE source = ?2 AND position = ?3",
                        params![resolved, source, position],
                    )?;
                }
            }
            self.search.apply(&changed, &removed)?;
            transaction.execute("UPDATE cache_state SET ready = 1", [])?;
            transaction.commit()?;
        }
        Ok(RefreshStats {
            upserted: changed.len(),
            removed: removed.len(),
            unchanged,
            elapsed: started.elapsed(),
        })
    }
}
