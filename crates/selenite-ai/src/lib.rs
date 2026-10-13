//! Opt-in BYO models and rebuildable hybrid retrieval over authoritative Markdown.

mod provider;
mod retrieval;

pub use provider::{AiConfig, ChatMessage, EmbeddingProvider, HttpProvider, LlmProvider};
pub use retrieval::{Answer, Citation, EmbeddingStats, ask, rebuild_embeddings, retrieve};
