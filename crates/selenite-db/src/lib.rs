//! Typed databases and saved views over authoritative Markdown pages.
//!
//! Schemas and views are TOML metadata; rows are always read from the vault.
//! Frontmatter edits preserve Markdown body bytes but may reserialize YAML
//! comments, ordering and scalar formatting. Writes use the core's compare-and-
//! swap filesystem boundary rather than a database-only content store.

mod model;
mod query;
mod store;

pub use model::{
    Cell, DatabaseSchema, DatabaseSource, FieldSchema, Filter, Layout, PropertyType, QueryResult,
    Row, RowGroup, SavedView, Sort, Stored,
};
pub use query::query_rows;
pub use store::DatabaseStore;
