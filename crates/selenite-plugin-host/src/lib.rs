//! Capability-scoped WebAssembly plugins and verified, configurable catalogs.
//!
//! The guest receives no WASI, network or ambient filesystem imports. Content
//! remains in Markdown and every permitted write uses core compare-and-swap.

mod catalog;
mod config;
pub mod pro;
mod runtime;
mod signed;

pub use catalog::{
    CATALOG_SIGNING_DOMAIN, Catalog, CatalogClient, CatalogEntry, InstalledPlugin, VerifiedCatalog,
    list_installed, load_installed, run_installed, run_installed_reviewed, verify_catalog,
};
pub use config::{Capability, PluginConfig, PluginManifest, SandboxLimits};
pub use runtime::{PluginHost, PluginRun};
pub use signed::SignedDocument;
