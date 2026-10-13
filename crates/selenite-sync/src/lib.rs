//! Selenite sync foundation.

/// Extension boundary for future sync providers.
pub trait SyncProvider {
    fn name(&self) -> &str;
}
