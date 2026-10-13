//! Selenite ai foundation.

/// Extension boundary for future language model providers.
pub trait LlmProvider {
    fn name(&self) -> &str;
}
