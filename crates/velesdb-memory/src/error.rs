//! Error type for the memory layer.

#[cfg(feature = "persistence")]
use velesdb_core::agent::AgentMemoryError;
use velesdb_core::Error as CoreError;

use crate::embedder::EmbedError;
use crate::extract::ExtractError;
use crate::rerank::RerankError;

/// The transport-neutral class of a [`MemoryError`] — the single source of
/// truth every adapter maps onto its own error channel (JSON-RPC code, napi
/// status, `PyO3` exception type), so the taxonomy can never drift between them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCategory {
    /// The caller supplied bad input (empty fact, reserved key, malformed
    /// filter) — a 4xx-style fault.
    InvalidInput,
    /// A referenced memory id does not exist.
    NotFound,
    /// An internal storage / embedding / extraction failure — a 5xx-style fault.
    Internal,
}

/// Errors returned by [`crate::service::MemoryService`].
#[derive(Debug, thiserror::Error)]
pub enum MemoryError {
    /// Failure in the underlying `VelesDB` storage engine.
    #[error("storage error: {0}")]
    Storage(#[from] CoreError),

    /// Failure in the Agent Memory SDK. Only constructible with the
    /// `persistence` feature (the native, file-backed store) — a
    /// `persistence`-free backend (e.g. `velesdb-wasm`'s in-memory one) never
    /// touches `velesdb-core`'s `agent` module, so this variant can't arise.
    #[cfg(feature = "persistence")]
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

    /// Failure reranking a fused-recall candidate pool in
    /// [`crate::service::MemoryService::recall_fused_reranked`].
    #[error("rerank error: {0}")]
    Rerank(#[from] RerankError),

    /// A fused-recall filter referenced a field name that is not a plain
    /// identifier, named a reserved key, or carried a non-scalar value.
    #[error("invalid filter field: {0}")]
    InvalidFilter(String),

    /// A relation label supplied to [`crate::service::MemoryService::relate`] or
    /// a [`crate::model::Link`] in
    /// [`crate::service::MemoryService::remember`] was invalid — empty, too long,
    /// or contained non-printable characters.
    #[error("invalid relation label: {0}")]
    InvalidRelation(String),

    /// A `remember` link failed after the fact was stored AND the
    /// compensating rollback delete also failed — unlike every other error
    /// from `remember`, the fact **remains stored**. Both errors are
    /// carried so the caller can see why the write failed and why the
    /// cleanup couldn't undo it.
    ///
    /// Neither field is `#[source]` — deliberately: the `Display` message
    /// already embeds both errors, and a source chain would double-print
    /// them in chain-style reports (anyhow, miette). Match on the variant
    /// to inspect the two errors programmatically.
    #[error(
        "link failed ({cause}); rollback delete also failed ({rollback}) — the fact remains stored"
    )]
    RollbackFailed {
        /// The link failure that triggered the rollback.
        cause: Box<MemoryError>,
        /// The storage failure that prevented the rollback delete.
        rollback: Box<MemoryError>,
    },
}

impl MemoryError {
    /// Classify this error into a transport-neutral [`ErrorCategory`]. Adapters
    /// map the *category*, not the variant, so the client-facing taxonomy stays
    /// identical across the MCP server and every binding.
    #[must_use]
    pub fn category(&self) -> ErrorCategory {
        match self {
            Self::EmptyFact
            | Self::ReservedKey(_)
            | Self::InvalidFilter(_)
            | Self::InvalidRelation(_) => ErrorCategory::InvalidInput,
            Self::UnknownMemory(_) => ErrorCategory::NotFound,
            #[cfg(feature = "persistence")]
            Self::Memory(_) => ErrorCategory::Internal,
            Self::Storage(_) | Self::Embed(_) | Self::Extract(_) | Self::Rerank(_) => {
                ErrorCategory::Internal
            }
            // The rollback failure is the storage-level fault that matters
            // to a client: the write is in an unexpected state.
            Self::RollbackFailed { .. } => ErrorCategory::Internal,
        }
    }
}
