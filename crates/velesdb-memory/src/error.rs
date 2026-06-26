//! Error type for the memory layer.

use velesdb_core::agent::AgentMemoryError;
use velesdb_core::Error as CoreError;

use crate::embedder::EmbedError;
use crate::extract::ExtractError;

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

    /// Caller metadata or a recall filter named a reserved key (`content` or a
    /// `_veles_`-prefixed system key), which callers may not set or filter on.
    #[error("metadata key '{0}' is reserved")]
    ReservedKey(String),

    /// Failure producing a text embedding.
    #[error("embedding error: {0}")]
    Embed(#[from] EmbedError),

    /// Failure extracting facts from raw text in
    /// [`crate::service::MemoryService::remember_extracted`].
    #[error("extraction error: {0}")]
    Extract(#[from] ExtractError),

    /// A fused-recall filter referenced a field name that is not a plain
    /// identifier, named a reserved key, or carried a non-scalar value.
    #[error("invalid filter field: {0}")]
    InvalidFilter(String),
}
