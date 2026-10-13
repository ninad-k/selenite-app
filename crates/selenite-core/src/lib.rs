//! Filesystem-independent Markdown primitives for a Selenite vault.

mod page;
pub mod vault;

pub use page::{Block, BlockKind, Page, ParseError, WikiLink, extract_links, validate_page_path};
