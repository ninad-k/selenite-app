use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use selenite_core::Page;
use tantivy::{
    Index, IndexReader, ReloadPolicy, TantivyDocument, TantivyError,
    collector::TopDocs,
    query::QueryParser,
    schema::{Field, STORED, STRING, Schema, TEXT, Value},
};

use super::{IncompatibleCache, path_from_key, path_key};

#[derive(Debug, Clone, PartialEq)]
pub struct SearchHit {
    pub path: PathBuf,
    pub title: String,
    pub score: f32,
}

pub(crate) struct SearchIndex {
    index: Index,
    reader: IndexReader,
    path: Field,
    title: Field,
    body: Field,
    new: bool,
}

impl SearchIndex {
    pub(crate) fn open(directory: &Path) -> Result<Self> {
        validate_directory(directory)?;
        let mut schema = Schema::builder();
        let path = schema.add_text_field("path", STRING | STORED);
        let title = schema.add_text_field("title", TEXT | STORED);
        let body = schema.add_text_field("body", TEXT);
        let schema = schema.build();
        let new = !directory.join("meta.json").exists();
        let index = if new {
            if fs::read_dir(directory)?.next().is_some() {
                return Err(IncompatibleCache)
                    .context("search metadata is missing from a nonempty cache");
            }
            Index::create_in_dir(directory, schema.clone()).map_err(classify_error)?
        } else {
            Index::open_in_dir(directory).map_err(classify_error)?
        };
        if index.schema() != schema {
            return Err(IncompatibleCache).context("search schema is incompatible");
        }
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::Manual)
            .try_into()
            .map_err(classify_error)?;
        Ok(Self {
            index,
            reader,
            path,
            title,
            body,
            new,
        })
    }

    pub(crate) fn is_new(&self) -> bool {
        self.new
    }

    pub(crate) fn reload(&self) -> Result<()> {
        self.reader.reload().map_err(classify_error)
    }

    pub(crate) fn replace_all(&mut self, pages: &[Page]) -> Result<()> {
        // This is a logical cache operation. No Markdown or user file is deleted.
        let mut writer = self.index.writer_with_num_threads(1, 50_000_000)?;
        writer.delete_all_documents()?;
        for page in pages {
            let mut document = TantivyDocument::default();
            document.add_text(self.path, path_key(&page.path));
            document.add_text(self.title, &page.title);
            document.add_text(self.body, &page.body);
            writer.add_document(document)?;
        }
        writer.commit()?;
        drop(writer);
        self.reader.reload()?;
        self.new = false;
        Ok(())
    }

    pub(crate) fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchHit>> {
        if query.trim().is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        let parser = QueryParser::for_index(&self.index, vec![self.title, self.body]);
        let query = parser.parse_query(query).context("invalid search query")?;
        let searcher = self.reader.searcher();
        let count = usize::try_from(searcher.num_docs()).context("search cache is too large")?;
        if count == 0 {
            return Ok(Vec::new());
        }
        // Collect all matches before truncating, so equal scores at the cutoff
        // cannot acquire a different order after rebuilding segment files.
        let matches = searcher.search(&query, &TopDocs::with_limit(count).order_by_score())?;
        let mut hits = Vec::with_capacity(matches.len());
        for (score, address) in matches {
            let document: TantivyDocument = searcher.doc(address).map_err(classify_error)?;
            let path = document
                .get_first(self.path)
                .and_then(|value| value.as_str())
                .context(IncompatibleCache)?;
            let title = document
                .get_first(self.title)
                .and_then(|value| value.as_str())
                .context(IncompatibleCache)?;
            hits.push(SearchHit {
                path: path_from_key(path),
                title: title.to_owned(),
                score,
            });
        }
        hits.sort_by(|left, right| {
            right
                .score
                .total_cmp(&left.score)
                .then_with(|| left.path.cmp(&right.path))
        });
        hits.truncate(limit);
        Ok(hits)
    }
}

fn classify_error(error: TantivyError) -> anyhow::Error {
    if matches!(
        error,
        TantivyError::DataCorruption(_)
            | TantivyError::SchemaError(_)
            | TantivyError::IncompatibleIndex(_)
    ) {
        anyhow::Error::new(error).context(IncompatibleCache)
    } else {
        error.into()
    }
}

/// Refuse links before Tantivy can follow them. All entries in this dedicated
/// disposable cache must be ordinary files or directories.
fn validate_directory(directory: &Path) -> Result<()> {
    match fs::symlink_metadata(directory) {
        Ok(metadata) => ensure!(
            metadata.is_dir() && !metadata.file_type().is_symlink(),
            "search cache is not a regular directory: {}",
            directory.display()
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => fs::create_dir(directory)?,
        Err(error) => return Err(error.into()),
    }
    let mut pending = vec![directory.to_owned()];
    while let Some(path) = pending.pop() {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            ensure!(
                !kind.is_symlink(),
                "search cache contains a symbolic link: {}",
                entry.path().display()
            );
            ensure!(
                kind.is_file() || kind.is_dir(),
                "search cache contains a non-regular entry: {}",
                entry.path().display()
            );
            if kind.is_dir() {
                pending.push(entry.path());
            }
        }
    }
    Ok(())
}
