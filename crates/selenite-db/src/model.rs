use std::{collections::BTreeSet, path::Path};

use anyhow::{Context, Result, bail, ensure};
use chrono::NaiveDate;
use selenite_core::{LinkResolver, ResolveError, vault::validate_relative};
use serde::{Deserialize, Serialize};
use serde_yaml::Value;

/// Identifiers are portable filename stems, not arbitrary filesystem paths.
pub(crate) fn validate_id(id: &str) -> Result<()> {
    ensure!(
        !id.is_empty()
            && id.len() <= 64
            && id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')),
        "definition ID must contain 1–64 ASCII letters, digits, hyphens or underscores"
    );
    let upper = id.to_ascii_uppercase();
    ensure!(
        !matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            && !(upper.len() == 4
                && (upper.starts_with("COM") || upper.starts_with("LPT"))
                && matches!(upper.as_bytes()[3], b'1'..=b'9')),
        "definition ID is a reserved Windows filename"
    );
    Ok(())
}

pub(crate) fn portable_path(path: &str) -> Result<()> {
    ensure!(
        !path.contains('\\'),
        "use forward slashes in serialized vault paths"
    );
    validate_relative(Path::new(path)).context("invalid vault-relative path")
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DatabaseSource {
    /// An empty path selects every page in the vault; other paths are recursive.
    Folder { path: String },
    /// Membership uses the YAML `tags` string or list, not body hashtags.
    Tag { tag: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PropertyType {
    Text,
    Number,
    Date,
    Select,
    MultiSelect,
    Checkbox,
    Relation,
    Url,
    File,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldSchema {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: PropertyType,
    #[serde(default)]
    pub required: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatabaseSchema {
    pub id: String,
    pub name: String,
    pub source: DatabaseSource,
    #[serde(default)]
    pub fields: Vec<FieldSchema>,
}

impl DatabaseSchema {
    pub fn validate(&self) -> Result<()> {
        validate_id(&self.id)?;
        ensure!(
            !self.name.trim().is_empty(),
            "database name cannot be empty"
        );
        match &self.source {
            DatabaseSource::Folder { path } if !path.is_empty() => portable_path(path)?,
            DatabaseSource::Tag { tag } => ensure!(
                !tag.is_empty()
                    && !tag.starts_with('#')
                    && !tag.chars().any(char::is_whitespace)
                    && !tag.chars().any(char::is_control),
                "tag must be a nonempty YAML tag without a leading # or whitespace"
            ),
            DatabaseSource::Folder { .. } => {}
        }
        let mut names = BTreeSet::new();
        for field in &self.fields {
            ensure!(
                !field.name.trim().is_empty()
                    && !field.name.starts_with('$')
                    && !field.name.chars().any(char::is_control),
                "invalid field name {:?}; $ names are reserved for page metadata",
                field.name
            );
            ensure!(names.insert(&field.name), "duplicate field {}", field.name);
            if matches!(field.kind, PropertyType::Select | PropertyType::MultiSelect) {
                ensure!(
                    !field.options.is_empty(),
                    "{} needs select options",
                    field.name
                );
                let mut options = BTreeSet::new();
                for option in &field.options {
                    ensure!(
                        !option.is_empty() && options.insert(option),
                        "{} has an empty or duplicate option",
                        field.name
                    );
                }
            } else {
                ensure!(
                    field.options.is_empty(),
                    "{} is not a selection field",
                    field.name
                );
            }
        }
        Ok(())
    }

    pub fn field(&self, name: &str) -> Result<&FieldSchema> {
        self.fields
            .iter()
            .find(|field| field.name == name)
            .with_context(|| format!("unknown field {name}"))
    }

    pub(crate) fn field_type(&self, name: &str) -> Result<PropertyType> {
        if matches!(name, "$title" | "$path") {
            Ok(PropertyType::Text)
        } else {
            Ok(self.field(name)?.kind)
        }
    }

    pub fn validate_cell(&self, name: &str, cell: &Cell) -> Result<()> {
        self.field(name)?.validate_cell(cell)
    }
}

/// The discriminator makes native/frontend values unambiguous. Empty removes a
/// frontmatter property; it is distinct from zero and an unchecked checkbox.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum Cell {
    Empty,
    Text(String),
    Number(f64),
    Date(String),
    Select(String),
    MultiSelect(Vec<String>),
    Checkbox(bool),
    /// Page references may be plain link targets or [[wikilinks]]. Missing
    /// targets remain valid references, just as they do in Markdown content.
    Relation(Vec<String>),
    Url(String),
    File(String),
}

impl Cell {
    pub fn is_empty(&self) -> bool {
        match self {
            Self::Empty => true,
            Self::Text(value)
            | Self::Date(value)
            | Self::Select(value)
            | Self::Url(value)
            | Self::File(value) => value.is_empty(),
            Self::MultiSelect(values) | Self::Relation(values) => values.is_empty(),
            Self::Number(_) | Self::Checkbox(_) => false,
        }
    }

    pub(crate) fn text(&self) -> Option<&str> {
        match self {
            Self::Text(value)
            | Self::Date(value)
            | Self::Select(value)
            | Self::Url(value)
            | Self::File(value) => Some(value),
            _ => None,
        }
    }

    pub(crate) fn yaml(&self) -> Result<Value> {
        Ok(match self {
            Self::Empty => Value::Null,
            Self::Text(value)
            | Self::Date(value)
            | Self::Select(value)
            | Self::Url(value)
            | Self::File(value) => Value::String(value.clone()),
            Self::Number(value) => serde_yaml::to_value(value)?,
            Self::Checkbox(value) => Value::Bool(*value),
            Self::MultiSelect(values) | Self::Relation(values) => {
                Value::Sequence(values.iter().cloned().map(Value::String).collect())
            }
        })
    }
}

impl FieldSchema {
    pub fn validate_cell(&self, cell: &Cell) -> Result<()> {
        ensure!(
            !self.required || !cell.is_empty(),
            "{} is required",
            self.name
        );
        if matches!(cell, Cell::Empty) {
            return Ok(());
        }
        let valid = match (self.kind, cell) {
            (PropertyType::Text, Cell::Text(_)) | (PropertyType::Checkbox, Cell::Checkbox(_)) => {
                true
            }
            (PropertyType::Number, Cell::Number(value)) => value.is_finite(),
            (PropertyType::Date, Cell::Date(value)) => valid_date(value),
            (PropertyType::Select, Cell::Select(value)) => self.options.contains(value),
            (PropertyType::MultiSelect, Cell::MultiSelect(values)) => {
                unique(values) && values.iter().all(|value| self.options.contains(value))
            }
            (PropertyType::Relation, Cell::Relation(values)) => {
                unique(values) && values.iter().all(|value| valid_relation(value))
            }
            (PropertyType::Url, Cell::Url(value)) => {
                !value.chars().any(char::is_whitespace)
                    && url::Url::parse(value).is_ok_and(|url| {
                        matches!(url.scheme(), "http" | "https") && url.host_str().is_some()
                    })
            }
            (PropertyType::File, Cell::File(value)) => portable_path(value).is_ok(),
            _ => false,
        };
        ensure!(valid, "invalid {:?} value for {}", self.kind, self.name);
        Ok(())
    }

    pub(crate) fn parse(&self, value: Option<&Value>) -> Result<Cell> {
        let cell = match value {
            None | Some(Value::Null) => Cell::Empty,
            Some(value) => match self.kind {
                PropertyType::Text => Cell::Text(string(value)?),
                PropertyType::Number => Cell::Number(value.as_f64().context("expected a number")?),
                PropertyType::Date => Cell::Date(string(value)?),
                PropertyType::Select => Cell::Select(string(value)?),
                PropertyType::MultiSelect => Cell::MultiSelect(strings(value, false)?),
                PropertyType::Checkbox => {
                    Cell::Checkbox(value.as_bool().context("expected a checkbox boolean")?)
                }
                PropertyType::Relation => Cell::Relation(strings(value, true)?),
                PropertyType::Url => Cell::Url(string(value)?),
                PropertyType::File => Cell::File(string(value)?),
            },
        };
        self.validate_cell(&cell)?;
        Ok(cell)
    }
}

fn string(value: &Value) -> Result<String> {
    value
        .as_str()
        .map(str::to_owned)
        .context("expected a string")
}

fn strings(value: &Value, allow_single: bool) -> Result<Vec<String>> {
    match value {
        Value::String(value) if allow_single => Ok(vec![value.clone()]),
        Value::Sequence(values) => values.iter().map(string).collect(),
        _ => bail!("expected a list of strings"),
    }
}

fn unique(values: &[String]) -> bool {
    values.iter().collect::<BTreeSet<_>>().len() == values.len()
}

pub(crate) fn valid_date(value: &str) -> bool {
    value.len() == 10
        && NaiveDate::parse_from_str(value, "%Y-%m-%d")
            .is_ok_and(|date| date.format("%Y-%m-%d").to_string() == value)
}

fn valid_relation(value: &str) -> bool {
    let target = value
        .strip_prefix("[[")
        .and_then(|value| value.strip_suffix("]]"))
        .unwrap_or(value);
    if target.trim().is_empty() {
        return false;
    }
    let resolver = LinkResolver::new(std::iter::empty::<&Path>()).expect("empty resolver is valid");
    // Validate syntax independently of the row's location. Actual resolution
    // uses the referring page and checks whether `..` escapes that vault root.
    let mut source = std::path::PathBuf::new();
    for _ in 0..target
        .split(['/', '\\'])
        .filter(|part| *part == "..")
        .count()
    {
        source.push("ancestor");
    }
    source.push("reference.md");
    !matches!(
        resolver.resolve(&source, target),
        Err(ResolveError::Invalid(_))
    )
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Filter {
    All {
        filters: Vec<Filter>,
    },
    Any {
        filters: Vec<Filter>,
    },
    Eq {
        field: String,
        value: Cell,
    },
    Contains {
        field: String,
        value: String,
    },
    /// Inclusive number/date bounds. At least one bound is required.
    Range {
        field: String,
        #[serde(default)]
        min: Option<Cell>,
        #[serde(default)]
        max: Option<Cell>,
    },
    IsEmpty {
        field: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sort {
    pub field: String,
    #[serde(default)]
    pub descending: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Layout {
    #[default]
    Table,
    Board,
    Calendar,
    Gallery,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedView {
    pub id: String,
    pub name: String,
    pub database: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter: Option<Filter>,
    #[serde(default)]
    pub sort: Vec<Sort>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_by: Option<String>,
    #[serde(default)]
    pub layout: Layout,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub calendar_field: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gallery_field: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Row {
    /// Portable vault-relative path using forward slashes.
    pub path: String,
    pub title: String,
    /// The exact source returned to the caller for compare-and-swap edits.
    pub source: String,
    pub cells: std::collections::BTreeMap<String, Cell>,
}

impl Row {
    pub(crate) fn cell(&self, name: &str) -> Cell {
        match name {
            "$title" => Cell::Text(self.title.clone()),
            "$path" => Cell::Text(self.path.clone()),
            _ => self.cells.get(name).cloned().unwrap_or(Cell::Empty),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RowGroup {
    pub key: Cell,
    /// Row paths in the same stable order as the query's rows.
    pub rows: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QueryResult {
    pub schema: DatabaseSchema,
    pub view: SavedView,
    pub rows: Vec<Row>,
    pub groups: Vec<RowGroup>,
}

/// A persisted definition plus its exact TOML for compare-and-swap saves.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stored<T> {
    pub definition: T,
    pub source: String,
}
