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
    /// [`crate::storage::AUTO_DATE_FIELD`] is the one documented exception:
    /// a caller MAY set it (e.g. to date a fact retroactively), so it never
    /// raises this error.
    #[error("metadata key '{0}' is reserved")]
    ReservedKey(String),

    /// Caller-supplied `metadata` (on `remember`/`remember_with_ttl` or a
    /// context-compiler fragment) exceeded [`crate::limits::MAX_METADATA_BYTES`]
    /// — a `DoS` guard, since metadata is a keyed lookup facet, not a payload.
    #[error("metadata of {bytes} bytes exceeds the cap of {max} bytes")]
    MetadataTooLarge {
        /// The serialized size of the rejected metadata, in bytes.
        bytes: usize,
        /// The cap that was exceeded ([`crate::limits::MAX_METADATA_BYTES`]).
        max: usize,
    },

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

    /// A context-compile request carried a token budget that cannot hold any
    /// context: zero, or not larger than the response reserve the policy
    /// keeps aside for the model's answer.
    #[cfg(feature = "context")]
    #[error("token budget {budget} cannot hold any context (reserve {reserve})")]
    ContextBudget {
        /// The caller-supplied token budget.
        budget: u64,
        /// The response reserve the policy subtracts from the budget.
        reserve: u64,
    },

    /// A context-compile request exceeded a resource cap from
    /// [`crate::limits`] — too many fragments, or one fragment larger than
    /// the per-fragment byte ceiling.
    #[cfg(feature = "context")]
    #[error("context request over limit: {0}")]
    ContextOverLimit(String),

    /// A transcript segmentation request failed because of the transcript's
    /// FORMAT, not its size — e.g. `segmentation.format: "jsonl"` forced on
    /// a line that does not parse as a `{role, content}` JSON object (see
    /// [`crate::context::segment::segment_transcript`]). Deliberately
    /// distinct from [`Self::ContextOverLimit`] (issue #1516, m2): a parsing
    /// failure is not a budget/cap breach, so a caller filtering on the
    /// error message no longer sees the misleading "over limit" wording for
    /// what is really a malformed-input error. Same
    /// [`ErrorCategory::InvalidInput`] classification as `ContextOverLimit`
    /// (both map to `INVALID_PARAMS` over MCP) — only the variant, and the
    /// message, differ.
    #[cfg(feature = "context")]
    #[error("transcript segmentation error: {0}")]
    SegmentationError(String),

    /// A `ctx://source/<hash>` handle was malformed or nothing is stored
    /// under it (the source was never stored, expired, or was forgotten).
    #[cfg(feature = "context")]
    #[error("unknown context source handle: {0}")]
    UnknownHandle(String),

    /// [`crate::service::MemoryService::explain_compilation`]'s
    /// `fragment_index` named a position beyond `request.fragments`.
    #[cfg(feature = "context")]
    #[error("fragment_index {index} is out of bounds: request.fragments has {len} entries")]
    FragmentIndexOutOfBounds {
        /// The out-of-bounds index the caller supplied.
        index: usize,
        /// The actual number of fragments in the request.
        len: usize,
    },

    /// [`crate::service::MemoryService::explain_compilation`] found no
    /// decision matching the requested `fragment_id` (and no
    /// `fragment_index` was given, or it selected nothing new to check).
    #[cfg(feature = "context")]
    #[error("the request contains no fragment with id {0}")]
    FragmentNotFound(u64),

    /// A persisted working context could not be (de)serialized — the stored
    /// payload predates or postdates this crate's schema.
    #[cfg(feature = "context")]
    #[error("working context codec error: {0}")]
    WorkingContextCodec(String),

    /// A context fragment carried a `path` (V2b-1 path ingestion) but no
    /// filesystem root is configured (`VELESDB_MEMORY_INGEST_ROOTS` unset or
    /// empty) — the tool is always advertised, but ingestion itself is
    /// opt-in. Also the fallback the pure compiler core reports when a
    /// `path` fragment reaches it unresolved (e.g. a binding that has no
    /// ingest adapter, such as the WASM build): [`crate::context`] never
    /// performs I/O itself, so an un-cleared `path` field always means the
    /// adapter that should have resolved or rejected it was skipped.
    #[cfg(feature = "context")]
    #[error(
        "path ingestion is disabled: set VELESDB_MEMORY_INGEST_ROOTS to enable the `path` field"
    )]
    IngestDisabled,

    /// A `path`-referenced fragment resolved (after following symlinks) to a
    /// location outside every configured ingest root. Carries the
    /// caller-supplied `path` VERBATIM, never the canonicalized target — the
    /// resolved location may be filesystem structure the caller has no
    /// business learning about (e.g. that a symlink escapes).
    #[cfg(feature = "context")]
    #[error("path '{0}' is outside the configured ingest roots")]
    IngestOutsideRoots(String),

    /// A `path`-referenced fragment could not be read for any reason other
    /// than escaping the ingest roots: a relative path (an MCP server's
    /// working directory is unpredictable, so only absolute paths are
    /// accepted), a path that does not exist or is not a plain file
    /// (directories are rejected), a `path` fragment combined with
    /// non-empty `content` or a `media` payload (exactly one of `path`,
    /// `content`, `media` is accepted), or a file whose bytes are not valid
    /// UTF-8.
    #[cfg(feature = "context")]
    #[error("cannot ingest path: {0}")]
    IngestPath(String),

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
            | Self::InvalidRelation(_)
            | Self::MetadataTooLarge { .. } => ErrorCategory::InvalidInput,
            #[cfg(feature = "context")]
            Self::ContextBudget { .. } | Self::ContextOverLimit(_) | Self::SegmentationError(_) => {
                ErrorCategory::InvalidInput
            }
            #[cfg(feature = "context")]
            Self::IngestDisabled | Self::IngestOutsideRoots(_) | Self::IngestPath(_) => {
                ErrorCategory::InvalidInput
            }
            #[cfg(feature = "context")]
            Self::FragmentIndexOutOfBounds { .. } | Self::FragmentNotFound(_) => {
                ErrorCategory::InvalidInput
            }
            #[cfg(feature = "context")]
            Self::UnknownHandle(_) => ErrorCategory::NotFound,
            #[cfg(feature = "context")]
            Self::WorkingContextCodec(_) => ErrorCategory::Internal,
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
