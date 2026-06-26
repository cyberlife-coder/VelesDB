//! Error type for the memory layer.

use velesdb_core::agent::AgentMemoryError;
use velesdb_core::Error as CoreError;

use crate::embedder::EmbedError;

/// Errors returned by [`crate::service::MemoryService`].
#[derive(Debug, thiserror::Error)]
pub enum MemoryError {
    /// Failure in the underlying `VelesDB` storage engine.
    #[error("storage error: {0}")]
    Storage(#[from] CoreError),

    /// Failure in the Agent Memory SDK.
    #[error("memory error: {0}")]
    Memory(#[from] AgentMemoryError),

    /// A fact was empty or whitespace-only.
    #[error("fact text must not be empty")]
    EmptyFact,

    /// A `remember` link or a `relate` endpoint referenced a memory id that
    /// does not exist.
    #[error("memory {0} does not exist")]
    UnknownMemory(u64),

    /// Failure producing a text embedding.
    #[error("embedding error: {0}")]
    Embed(#[from] EmbedError),
}
