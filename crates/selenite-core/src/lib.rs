//! Filesystem-independent Markdown primitives for a Selenite vault.

mod links;
mod page;
mod persistence;
pub mod vault;

pub use links::{
    Backlink, BacklinkMap, LinkError, LinkResolver, RenamePlan, ResolveError, plan_rename,
};
pub use page::{Block, BlockKind, Page, ParseError, WikiLink, extract_links, validate_page_path};
pub use persistence::{RenameReport, WriteReport};
