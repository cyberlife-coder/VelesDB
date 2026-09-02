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
/// `non_exhaustive` so a future category is a wildcard arm downstream instead
/// of a breaking release. That trades away the compile-time exhaustiveness the
/// in-repo adapters relied on, so [`ErrorCategory::ALL`] restores it as a
/// test-time guard: each adapter iterates `ALL` and asserts its mapping is
/// total, which turns "someone added a category" from a silent fallback into
/// a red test naming the unmapped variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ErrorCategory {
    /// The caller supplied bad input (empty fact, reserved key, malformed
    /// filter) — a 4xx-style fault.
    InvalidInput,
    /// A referenced memory id does not exist.
    NotFound,
    /// An internal storage / embedding / extraction failure — a 5xx-style fault.
    Internal,
    /// The operation is not supported by the storage backend in use — a
    /// capability gap, not a caller mistake and not a fault. Introduced with
    /// the facet split (#1959) so a backend's honest refusal stops being
    /// billed to the client as invalid input.
    Unsupported,
}

impl ErrorCategory {
    /// Every category, for adapter coverage tests — see the type-level doc.
    ///
    /// Lives here because only the defining crate can enumerate a
    /// `non_exhaustive` enum; an adapter hand-listing the variants would just
    /// re-create the drift this exists to catch. Adding a variant without
    /// extending this slice fails `all_lists_every_category` below.
    pub const ALL: &'static [Self] = &[
        Self::InvalidInput,
        Self::NotFound,
        Self::Internal,
        Self::Unsupported,
    ];
}

/// Errors returned by [`crate::service::MemoryService`].
///
/// `non_exhaustive`: adapters classify through [`MemoryError::category`], never
/// by variant, so new variants must be a non-event downstream — which is also
/// what lets a variant's payload gain structure one minor release at a time
/// instead of in one breaking batch.
///
/// # `String` payloads are a decision here, not a debt
///
/// Every adapter consumes this type through [`MemoryError::category`] plus
/// `Display`; no payload is read programmatically outside this crate. So a
/// variant earns a structured payload only when structure is being **lost** —
/// a source error flattened out of the `source()` chain, or an in-crate
/// consumer parsing prose — and [`Self::WorkingContextCodec`] is the one that
/// qualified (it had both). The others carry prose on purpose: their messages
/// are heterogeneous narratives written for the person reading them
/// ([`Self::IngestPath`] additionally cites the *requested* path and never the
/// canonical one, a security decision its module documents), and forcing one
/// struct template over them would flatten exactly the nuance they exist to
/// deliver. Re-litigating this per variant is what this paragraph is for.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
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

    /// A fact was longer than [`crate::limits::MAX_EMBEDDABLE_TEXT_BYTES`],
    /// the size an embedding model still accepts. Refused BEFORE the embedder
    /// is called, so the caller learns the limit and its own size instead of
    /// an opaque backend fault (`ollama embeddings call failed`).
    #[error(
        "fact of {bytes} bytes exceeds the embeddable cap of {max} bytes: split it into several \
         shorter facts, or compile the long text with `compile_context` and remember a summary"
    )]
    FactTooLarge {
        /// The size of the rejected fact, in bytes.
        bytes: usize,
        /// The cap that was exceeded ([`crate::limits::MAX_EMBEDDABLE_TEXT_BYTES`]).
        max: usize,
    },

    /// [`crate::service::MemoryService::remember_with_ttl`] was given
    /// `Some(0)`. An explicit per-call `0` used to be normalised to "no
    /// expiry", i.e. a caller who meant "expire immediately" silently got a
    /// **permanent** fact — the exact opposite intent, with no signal. A TTL
    /// supplied as *configuration* (`with_default_ttl`, a compile policy's
    /// `source_ttl_seconds`) still reads `0` as "no TTL policy": that is a
    /// default, not an intent about one fact.
    #[error(
        "ttl_seconds must be greater than 0: omit it to store the fact permanently, or pass the \
         number of seconds the fact should live"
    )]
    ZeroTtl,

    /// [`crate::service::MemoryService::relate`] was asked to link a memory to
    /// itself. A self-loop states nothing and is traversed by `why` like any
    /// other edge, so it only adds noise to the evidence trail.
    #[error(
        "a memory cannot relate to itself (both endpoints are {0}): pass two different ids, or \
         record the property in the fact's own metadata"
    )]
    SelfRelation(u64),

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

    /// The online-migration observer could not durably classify a mutation.
    /// The source write has not run when this error is returned.
    #[cfg(feature = "persistence")]
    #[error("migration capture error: {0}")]
    MigrationCapture(String),

    /// The storage backend in use does not support the requested operation.
    /// A static description, not prose: the set of refusable operations is
    /// closed and known at compile time, and adapters display it verbatim.
    #[error("unsupported by this storage backend: {0}")]
    Unsupported(&'static str),

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

    /// [`crate::service::MemoryService::save_working_context`] was given a
    /// `WorkingContext` with nothing in it. The write is an idempotent upsert,
    /// so an empty save would *replace* — that is, destroy — the rich state
    /// already stored under this project and session. The one tool whose whole
    /// job is surviving a context loss must not be able to cause one on a call
    /// that carries nothing.
    #[cfg(feature = "context")]
    #[error(
        "working context is empty: fill at least one of goal, active_constraints, verified_facts, \
         open_hypotheses, decisions, exact_evidence or pending_actions — saving an empty state \
         would replace whatever is already stored under this project and session"
    )]
    EmptyWorkingContext,

    /// [`crate::service::MemoryService::save_working_context`] was given an
    /// empty (or whitespace-only) `project` or `session`. Both are the KEY the
    /// state is filed under and listed by: an empty one is accepted by every
    /// hash and every JSON encoder, so without this check it would save
    /// fine, list as `""`, and be unrecoverable by anyone who does not think
    /// to ask for the empty string. Refused before anything is written.
    #[cfg(feature = "context")]
    #[error("working-context {key} is empty: it is the id this state is saved and listed under")]
    EmptyWorkingContextKey {
        /// Which key was empty: `"project"` or `"session"`.
        key: &'static str,
    },

    /// A persisted working context could not be (de)serialized — the stored
    /// payload predates or postdates this crate's schema.
    ///
    /// The one String-payload variant that gained structure, because it is
    /// the one that had lost some: half its construction sites flattened a
    /// `serde_json::Error` into prose, destroying the `source()` chain
    /// `thiserror` exists to preserve — and it is also the only variant
    /// matched programmatically (the index reader falls back to an empty
    /// index on it rather than failing a load). `source` is `None` where the
    /// corruption is structural (a marker with no body) rather than a codec
    /// refusal.
    #[cfg(feature = "context")]
    #[error("working context codec error: {detail}")]
    WorkingContextCodec {
        /// What was being encoded or decoded, and for which slot.
        detail: String,
        /// The codec's own refusal, when there is one to preserve.
        #[source]
        source: Option<Box<serde_json::Error>>,
    },

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
    /// non-empty `content` or a `media` payload (`path` is exclusive,
    /// though `content` and `media` may travel together), a fragment
    /// carrying none of the three, or a file whose bytes are not valid
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
            | Self::FactTooLarge { .. }
            | Self::ZeroTtl
            | Self::SelfRelation(_)
            | Self::MetadataTooLarge { .. } => ErrorCategory::InvalidInput,
            Self::Unsupported(_) => ErrorCategory::Unsupported,
            #[cfg(feature = "context")]
            Self::EmptyWorkingContext | Self::EmptyWorkingContextKey { .. } => {
                ErrorCategory::InvalidInput
            }
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
            Self::WorkingContextCodec { .. } => ErrorCategory::Internal,
            Self::UnknownMemory(_) => ErrorCategory::NotFound,
            #[cfg(feature = "persistence")]
            Self::Memory(_) | Self::MigrationCapture(_) => ErrorCategory::Internal,
            Self::Storage(_) | Self::Embed(_) | Self::Extract(_) | Self::Rerank(_) => {
                ErrorCategory::Internal
            }
            // The rollback failure is the storage-level fault that matters
            // to a client: the write is in an unexpected state.
            Self::RollbackFailed { .. } => ErrorCategory::Internal,
        }
    }
}

#[cfg(test)]
#[path = "error_tests.rs"]
mod error_tests;
