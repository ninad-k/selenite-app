use std::{
    collections::BTreeMap,
    io,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use selenite_core::{
    LinkResolver, MetadataSnapshot, Page, RenamePlan, validate_page_path, vault::Vault,
};
use serde_yaml::{Mapping, Value};

use crate::{
    Cell, DatabaseSchema, DatabaseSource, PropertyType, QueryResult, Row, SavedView, Stored,
    model::{portable_path, validate_id},
    query_rows,
};

/// Metadata and row access through the core's safe filesystem boundary.
pub struct DatabaseStore {
    vault: Vault,
}

/// Page changes and the exact schema inputs used to derive them. Pass both to
/// the core's metadata-checked rename so a schema edit cannot race publication.
#[derive(Debug, Clone)]
pub struct ReviewedRenamePlan {
    pub plan: RenamePlan,
    pub schema_snapshot: MetadataSnapshot,
}

impl DatabaseStore {
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        Ok(Self::new(Vault::open(root)?))
    }

    pub fn new(vault: Vault) -> Self {
        Self { vault }
    }

    pub fn vault(&self) -> &Vault {
        &self.vault
    }

    /// Plan Markdown and schema-typed relation changes together. Membership and
    /// field types are taken from the original pages; text and unknown YAML
    /// properties are never inferred to be relations. Ambiguous targets remain
    /// untouched. For persistence use `plan_rename_reviewed` so metadata can be
    /// checked together with page publication.
    pub fn plan_rename(&self, pages: &[Page], from: &Path, to: &Path) -> Result<RenamePlan> {
        Ok(self.plan_rename_reviewed(pages, from, to)?.plan)
    }

    /// Capture schemas and derive relation edits while holding one shared lock.
    /// Publication must recheck `schema_snapshot` under the core write lock.
    pub fn plan_rename_reviewed(
        &self,
        pages: &[Page],
        from: &Path,
        to: &Path,
    ) -> Result<ReviewedRenamePlan> {
        let _guard = self.vault.lock_shared()?;
        let schema_snapshot = self.vault.metadata_snapshot("schemas")?;
        let schemas = schema_snapshot
            .files
            .iter()
            .filter(|(path, _)| {
                path.parent() == Some(Path::new("schemas"))
                    && path.extension().and_then(|part| part.to_str()) == Some("toml")
            })
            .map(|(path, source)| {
                let id = path
                    .file_stem()
                    .and_then(|part| part.to_str())
                    .context("schema name must be Unicode")?;
                parse_schema(id, source)
            })
            .collect::<Result<Vec<_>>>()?;
        let originals = pages
            .iter()
            .map(|page| Page::parse(&page.path, &page.source))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut plan = selenite_core::plan_rename(&originals, from, to)?;
        let before = LinkResolver::new(originals.iter().map(|page| &page.path))?;
        let after = LinkResolver::new(
            originals
                .iter()
                .map(|page| if page.path == from { to } else { &page.path }),
        )?;
        let mut changed: BTreeMap<PathBuf, Page> = plan
            .pages
            .into_iter()
            .map(|page| (page.path.clone(), page))
            .collect();
        for original in &originals {
            let target = if original.path == from {
                to
            } else {
                &original.path
            };
            let base = changed.get(target).unwrap_or(original);
            let mut frontmatter = base.frontmatter.clone();
            let applicable: Vec<_> = schemas
                .iter()
                .filter(|schema| belongs(schema, original))
                .collect();
            let mut relation_fields = BTreeMap::new();
            for schema in &applicable {
                for field in &schema.fields {
                    if field.kind != PropertyType::Relation {
                        continue;
                    }
                    ensure!(
                        applicable
                            .iter()
                            .flat_map(|schema| &schema.fields)
                            .filter(|other| other.name == field.name)
                            .all(|other| other.kind == PropertyType::Relation),
                        "{} property {} has conflicting relation/text schemas",
                        original.path.display(),
                        field.name
                    );
                    relation_fields.entry(field.name.clone()).or_insert(field);
                }
            }
            for (name, field) in relation_fields {
                let key = Value::String(name);
                let Some(value) = frontmatter.get(&key) else {
                    continue;
                };
                if value.is_null() {
                    continue;
                }
                let Cell::Relation(values) = field.parse(Some(value)).with_context(|| {
                    format!("{} property {}", original.path.display(), field.name)
                })?
                else {
                    unreachable!("relation fields parse as relations");
                };
                let rewritten = values
                    .iter()
                    .map(|value| {
                        rewrite_relation(&before, &after, &original.path, target, from, to, value)
                    })
                    .collect::<Result<Vec<_>>>()?;
                if rewritten == values {
                    continue;
                }
                field.validate_cell(&Cell::Relation(rewritten.clone()))?;
                let replacement = if value.is_string() {
                    Value::String(
                        rewritten
                            .into_iter()
                            .next()
                            .expect("scalar relation has one value"),
                    )
                } else {
                    Value::Sequence(rewritten.into_iter().map(Value::String).collect())
                };
                frontmatter.insert(key, replacement);
            }
            if frontmatter != base.frontmatter {
                let source = with_frontmatter(base, &frontmatter)?;
                changed.insert(target.to_owned(), Page::parse(target, &source)?);
            }
        }
        plan.pages = changed.into_values().collect();
        Ok(ReviewedRenamePlan {
            plan,
            schema_snapshot,
        })
    }

    pub fn load_schema(&self, id: &str) -> Result<Option<DatabaseSchema>> {
        Ok(self
            .load_schema_document(id)?
            .map(|document| document.definition))
    }

    pub fn load_schema_document(&self, id: &str) -> Result<Option<Stored<DatabaseSchema>>> {
        let Some(source) = self.vault.read_metadata(definition_path("schemas", id)?)? else {
            return Ok(None);
        };
        let definition = parse_schema(id, &source)?;
        Ok(Some(Stored { definition, source }))
    }

    pub fn save_schema(&self, schema: &DatabaseSchema, expected_toml: Option<&str>) -> Result<()> {
        schema.validate()?;
        let source = toml::to_string_pretty(schema)?;
        self.vault.write_metadata(
            definition_path("schemas", &schema.id)?,
            expected_toml,
            &source,
        )?;
        Ok(())
    }

    pub fn list_schemas(&self) -> Result<Vec<DatabaseSchema>> {
        self.definition_ids("schemas")?
            .into_iter()
            .map(|id| {
                self.load_schema(&id)?
                    .with_context(|| format!("schema {id} disappeared while listing"))
            })
            .collect()
    }

    pub fn load_view(&self, id: &str) -> Result<Option<SavedView>> {
        Ok(self
            .load_view_document(id)?
            .map(|document| document.definition))
    }

    pub fn load_view_document(&self, id: &str) -> Result<Option<Stored<SavedView>>> {
        let Some(source) = self.vault.read_metadata(definition_path("views", id)?)? else {
            return Ok(None);
        };
        let definition: SavedView =
            toml::from_str(&source).with_context(|| format!("parse view {id}"))?;
        ensure!(definition.id == id, "view ID does not match its filename");
        let schema = self
            .load_schema(&definition.database)?
            .context("view refers to a missing schema")?;
        definition.validate(&schema)?;
        Ok(Some(Stored { definition, source }))
    }

    pub fn save_view(&self, view: &SavedView, expected_toml: Option<&str>) -> Result<()> {
        let schema = self
            .load_schema(&view.database)?
            .context("view refers to a missing schema")?;
        view.validate(&schema)?;
        let source = toml::to_string_pretty(view)?;
        self.vault
            .write_metadata(definition_path("views", &view.id)?, expected_toml, &source)?;
        Ok(())
    }

    pub fn list_views(&self) -> Result<Vec<SavedView>> {
        self.definition_ids("views")?
            .into_iter()
            .map(|id| {
                self.load_view(&id)?
                    .with_context(|| format!("view {id} disappeared while listing"))
            })
            .collect()
    }

    fn definition_ids(&self, directory: &str) -> Result<Vec<String>> {
        let mut ids = Vec::new();
        for path in self.vault.metadata_paths(directory)? {
            // Definitions are flat. Other user metadata remains untouched.
            if path.parent() != Some(Path::new(directory))
                || path.extension().and_then(|part| part.to_str()) != Some("toml")
            {
                continue;
            }
            let id = path
                .file_stem()
                .and_then(|part| part.to_str())
                .context("definition name must be Unicode")?;
            validate_id(id)?;
            ids.push(id.to_owned());
        }
        ids.sort();
        Ok(ids)
    }

    pub fn rows(&self, schema: &DatabaseSchema) -> Result<Vec<Row>> {
        schema.validate()?;
        let _source_guard = self.vault.lock_shared()?;
        let mut rows = Vec::new();
        for path in self.vault.markdown_paths()? {
            if let DatabaseSource::Folder { path: folder } = &schema.source
                && !folder.is_empty()
                && !path.starts_with(path_from_key(folder))
            {
                continue;
            }
            let source = self
                .vault
                .read_source(&path)
                .with_context(|| format!("read {}", path.display()))?;
            let page =
                Page::parse(&path, &source).with_context(|| format!("parse {}", path.display()))?;
            if belongs(schema, &page) {
                rows.push(row_from_page(schema, &page)?);
            }
        }
        Ok(rows)
    }

    pub fn query(&self, schema: &DatabaseSchema, view: &SavedView) -> Result<QueryResult> {
        view.validate(schema)?;
        query_rows(schema, view, self.rows(schema)?)
    }

    pub fn query_saved(&self, view_id: &str) -> Result<QueryResult> {
        let view = self
            .load_view(view_id)?
            .context("saved view does not exist")?;
        let schema = self
            .load_schema(&view.database)?
            .context("database schema does not exist")?;
        self.query(&schema, &view)
    }

    /// Edit configured properties only. Unknown YAML properties and Markdown
    /// body bytes survive; YAML comments and formatting may be reserialized.
    pub fn update_row(
        &self,
        schema: &DatabaseSchema,
        path: &str,
        expected_source: &str,
        changes: &BTreeMap<String, Cell>,
    ) -> Result<Row> {
        schema.validate()?;
        portable_path(path)?;
        let path = path_from_key(path);
        let current = self.vault.read_source(&path)?;
        if current != expected_source {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "page changed since it was read",
            )
            .into());
        }
        let page = Page::parse(&path, &current)?;
        ensure!(
            belongs(schema, &page),
            "page does not belong to this database"
        );
        let mut frontmatter = page.frontmatter.clone();
        apply_cells(schema, &mut frontmatter, changes)?;
        let source = if frontmatter == page.frontmatter {
            current
        } else {
            with_frontmatter(&page, &frontmatter)?
        };
        let updated = Page::parse(&path, &source)?;
        let row = row_from_page(schema, &updated)?;
        // The core rechecks expected bytes under its write lock, including for
        // no-op edits, so concurrent external edits never become lost updates.
        self.vault.write_page(&path, expected_source, &source)?;
        Ok(row)
    }

    pub fn create_row(
        &self,
        schema: &DatabaseSchema,
        path: &str,
        body: &str,
        cells: &BTreeMap<String, Cell>,
    ) -> Result<Row> {
        schema.validate()?;
        portable_path(path)?;
        let path = path_from_key(path);
        validate_page_path(&path)?;
        let mut frontmatter = Mapping::new();
        apply_cells(schema, &mut frontmatter, cells)?;
        if let DatabaseSource::Tag { tag } = &schema.source {
            add_tag(&mut frontmatter, tag)?;
        }
        // Treat the supplied body as Markdown content, even when it begins
        // with a thematic break. It is never parsed as supplied frontmatter.
        let source = format!("---\n{}---\n{body}", serde_yaml::to_string(&frontmatter)?);
        let page = Page::parse(&path, &source)?;
        ensure!(
            belongs(schema, &page),
            "new page path does not belong to the database folder"
        );
        let row = row_from_page(schema, &page)?;
        ensure!(
            self.vault.create_page(&path, &source)?,
            "page already exists; creation never overwrites it"
        );
        Ok(row)
    }

    /// Copy a named built-in metadata template only if it is absent. Existing
    /// files are preserved; the operation is restartable, not a multi-file transaction.
    pub fn install_gtd_templates(&self) -> Result<Vec<String>> {
        let mut created = Vec::new();
        for (path, source) in GTD_TEMPLATES {
            if self.vault.read_metadata(path)?.is_none() {
                match self.vault.write_metadata(path, None, source) {
                    Ok(()) => created.push((*path).to_owned()),
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                    Err(error) => return Err(error.into()),
                }
            }
        }
        Ok(created)
    }
}

fn definition_path(directory: &str, id: &str) -> Result<PathBuf> {
    validate_id(id)?;
    Ok(Path::new(directory).join(format!("{id}.toml")))
}

fn parse_schema(id: &str, source: &str) -> Result<DatabaseSchema> {
    validate_id(id)?;
    let definition: DatabaseSchema =
        toml::from_str(source).with_context(|| format!("parse schema {id}"))?;
    definition.validate()?;
    ensure!(definition.id == id, "schema ID does not match its filename");
    Ok(definition)
}

fn path_from_key(key: &str) -> PathBuf {
    key.split('/').collect()
}

fn path_key(path: &Path) -> String {
    path.components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn rewrite_relation(
    before: &LinkResolver,
    after: &LinkResolver,
    original_source: &Path,
    new_source: &Path,
    from: &Path,
    to: &Path,
    value: &str,
) -> Result<String> {
    let wrapped = value
        .strip_prefix("[[")
        .and_then(|value| value.strip_suffix("]]"));
    let target = wrapped.unwrap_or(value);
    let Ok(previous) = before.resolve(original_source, target) else {
        return Ok(value.to_owned());
    };
    let desired = if previous == from { to } else { &previous };
    if after.resolve(new_source, target).as_deref() == Ok(desired) {
        return Ok(value.to_owned());
    }
    let include_extension = target.to_ascii_lowercase().ends_with(".md");
    let replacement = if desired
        .parent()
        .is_some_and(|parent| !parent.as_os_str().is_empty())
    {
        let mut key = path_key(desired);
        if !include_extension {
            key.truncate(key.len() - 3);
        }
        key
    } else {
        let depth = new_source
            .parent()
            .map_or(0, |parent| parent.components().count());
        let name = if include_extension {
            desired.file_name()
        } else {
            desired.file_stem()
        }
        .and_then(|part| part.to_str())
        .context("invalid relation destination")?;
        format!(
            "{}{name}",
            if depth == 0 {
                "./".into()
            } else {
                "../".repeat(depth)
            }
        )
    };
    ensure!(
        after.resolve(new_source, &replacement).as_deref() == Ok(desired),
        "cannot preserve relation target for {}",
        original_source.display()
    );
    Ok(if wrapped.is_some() {
        format!("[[{replacement}]]")
    } else {
        replacement
    })
}

fn belongs(schema: &DatabaseSchema, page: &Page) -> bool {
    match &schema.source {
        DatabaseSource::Folder { path } => {
            path.is_empty() || page.path.starts_with(path_from_key(path))
        }
        DatabaseSource::Tag { tag } => match page.frontmatter.get(Value::String("tags".into())) {
            Some(Value::String(value)) => value.trim_start_matches('#') == tag,
            Some(Value::Sequence(values)) => values.iter().any(|value| {
                value
                    .as_str()
                    .is_some_and(|value| value.trim_start_matches('#') == tag)
            }),
            _ => false,
        },
    }
}

fn row_from_page(schema: &DatabaseSchema, page: &Page) -> Result<Row> {
    ensure!(
        page.frontmatter.keys().all(|key| key.as_str().is_some()),
        "{} has non-string YAML property names",
        page.path.display()
    );
    let resolver = LinkResolver::new(std::iter::empty::<&Path>())?;
    let cells = schema
        .fields
        .iter()
        .map(|field| {
            let value = field
                .parse(page.frontmatter.get(Value::String(field.name.clone())))
                .with_context(|| format!("{} property {}", page.path.display(), field.name))?;
            if let Cell::Relation(values) = &value {
                for value in values {
                    let target = value
                        .strip_prefix("[[")
                        .and_then(|value| value.strip_suffix("]]"))
                        .unwrap_or(value);
                    ensure!(
                        !matches!(
                            resolver.resolve(&page.path, target),
                            Err(selenite_core::ResolveError::Invalid(_))
                        ),
                        "{} property {} has a relation path outside the vault",
                        page.path.display(),
                        field.name
                    );
                }
            }
            Ok((field.name.clone(), value))
        })
        .collect::<Result<_>>()?;
    Ok(Row {
        path: path_key(&page.path),
        title: page.title.clone(),
        source: page.source.clone(),
        cells,
    })
}

fn apply_cells(
    schema: &DatabaseSchema,
    frontmatter: &mut Mapping,
    changes: &BTreeMap<String, Cell>,
) -> Result<()> {
    for (name, value) in changes {
        schema.validate_cell(name, value)?;
        let key = Value::String(name.clone());
        if matches!(value, Cell::Empty) {
            frontmatter.remove(&key);
        } else {
            frontmatter.insert(key, value.yaml()?);
        }
    }
    Ok(())
}

fn with_frontmatter(page: &Page, frontmatter: &Mapping) -> Result<String> {
    let newline = if page
        .source
        .split_once('\n')
        .is_some_and(|(line, _)| line.ends_with('\r'))
    {
        "\r\n"
    } else {
        "\n"
    };
    let yaml = serde_yaml::to_string(frontmatter)?.replace('\n', newline);
    let bom = if page.source.starts_with('\u{feff}') {
        "\u{feff}"
    } else {
        ""
    };
    Ok(format!("{bom}---{newline}{yaml}---{newline}{}", page.body))
}

fn add_tag(frontmatter: &mut Mapping, tag: &str) -> Result<()> {
    let key = Value::String("tags".into());
    let mut tags = match frontmatter.get(&key) {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::String(value)) => vec![Value::String(value.clone())],
        Some(Value::Sequence(values)) => values.clone(),
        _ => anyhow::bail!("tags must be a string or list of strings"),
    };
    ensure!(
        tags.iter().all(|value| value.as_str().is_some()),
        "tags must contain only strings"
    );
    if !tags.iter().any(|value| {
        value
            .as_str()
            .is_some_and(|value| value.trim_start_matches('#') == tag)
    }) {
        tags.push(Value::String(tag.to_owned()));
    }
    frontmatter.insert(key, Value::Sequence(tags));
    Ok(())
}

const GTD_TEMPLATES: &[(&str, &str)] = &[
    (
        "schemas/gtd.toml",
        include_str!("../../../templates/gtd/schemas/gtd.toml"),
    ),
    (
        "views/inbox.toml",
        include_str!("../../../templates/gtd/views/inbox.toml"),
    ),
    (
        "views/next_actions.toml",
        include_str!("../../../templates/gtd/views/next_actions.toml"),
    ),
    (
        "views/projects.toml",
        include_str!("../../../templates/gtd/views/projects.toml"),
    ),
    (
        "views/waiting.toml",
        include_str!("../../../templates/gtd/views/waiting.toml"),
    ),
    (
        "views/someday.toml",
        include_str!("../../../templates/gtd/views/someday.toml"),
    ),
    (
        "views/weekly_review.toml",
        include_str!("../../../templates/gtd/views/weekly_review.toml"),
    ),
];
