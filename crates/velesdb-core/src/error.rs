//! Error types for `VelesDB`.
//!
//! This module provides a unified error type for all `VelesDB` operations,
//! designed for professional API exposure to Python/Node clients.

use thiserror::Error;

/// Result type alias for `VelesDB` operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors that can occur in `VelesDB` operations.
///
/// Each variant includes a descriptive error message suitable for end-users.
/// Error codes follow the pattern `VELES-XXX` for easy debugging.
#[derive(Error, Debug)]
#[non_exhaustive]
pub enum Error {
    /// Collection already exists (VELES-001).
    #[error("[VELES-001] Collection '{0}' already exists")]
    CollectionExists(String),

    /// Collection not found (VELES-002).
    #[error("[VELES-002] Collection '{0}' not found")]
    CollectionNotFound(String),

    /// Point not found (VELES-003).
    #[error("[VELES-003] Point with ID '{0}' not found")]
    PointNotFound(u64),

    /// Dimension mismatch (VELES-004).
    #[error("[VELES-004] Vector dimension mismatch: expected {expected}, got {actual}")]
    DimensionMismatch {
        /// Expected dimension.
        expected: usize,
        /// Actual dimension.
        actual: usize,
    },

    /// Invalid vector (VELES-005).
    #[error("[VELES-005] Invalid vector: {0}")]
    InvalidVector(String),

    /// Storage error (VELES-006).
    #[error("[VELES-006] Storage error: {0}")]
    Storage(String),

    /// Index error (VELES-007).
    #[error("[VELES-007] Index error: {0}")]
    Index(String),

    /// Index corrupted (VELES-008).
    ///
    /// Indicates that index files are corrupted and need to be rebuilt.
    #[error("[VELES-008] Index corrupted: {0}")]
    IndexCorrupted(String),

    /// Configuration error (VELES-009).
    #[error("[VELES-009] Configuration error: {0}")]
    Config(String),

    /// Query parsing error (VELES-010).
    ///
    /// Wraps `VelesQL` parse errors with position and context information.
    #[error("[VELES-010] Query error: {0}")]
    Query(String),

    /// IO error (VELES-011).
    #[error("[VELES-011] IO error: {0}")]
    Io(#[from] std::io::Error),

    /// Serialization error (VELES-012).
    #[error("[VELES-012] Serialization error: {0}")]
    Serialization(String),

    /// Internal error (VELES-013).
    ///
    /// Indicates an unexpected internal error. Please report if encountered.
    #[error("[VELES-013] Internal error: {0}")]
    Internal(String),

    /// A vector given to a write that stores none: into a collection that
    /// stores no vectors — a metadata collection, a graph without embeddings
    /// — through any write surface, the collection API or DML; or to
    /// `upsert_metadata` on a vector collection, which writes payloads only
    /// (VELES-014). Carries the name of the collection written to.
    #[error("[VELES-014] Vector not allowed on '{0}': this write stores no vector")]
    VectorNotAllowed(String),

    /// Vector search, or a query vector, against a collection that stores
    /// no vectors — a metadata collection, a graph without embeddings
    /// (VELES-015). Carries the name of what was searched.
    #[error("[VELES-015] Vector search not supported on '{0}': it stores no vectors. Use query() instead.")]
    SearchNotSupported(String),

    /// Vector required for vector collection (VELES-016).
    #[error("[VELES-016] Vector required for collection '{0}' (not metadata-only)")]
    VectorRequired(String),

    /// Schema validation error (VELES-017).
    #[error("[VELES-017] Schema validation error: {0}")]
    SchemaValidation(String),

    /// Graph operation not supported (VELES-018).
    #[error("[VELES-018] Graph operation not supported: {0}")]
    GraphNotSupported(String),

    /// Edge already exists (VELES-019).
    #[error("[VELES-019] Edge with ID '{0}' already exists")]
    EdgeExists(u64),

    /// Edge not found (VELES-020).
    #[error("[VELES-020] Edge with ID '{0}' not found")]
    EdgeNotFound(u64),

    /// Invalid edge label (VELES-021).
    #[error("[VELES-021] Invalid edge label: {0}")]
    InvalidEdgeLabel(String),

    /// Node not found (VELES-022).
    #[error("[VELES-022] Node with ID '{0}' not found")]
    NodeNotFound(u64),

    /// Numeric overflow (VELES-023).
    ///
    /// Indicates a numeric conversion would overflow or truncate.
    /// Use `try_from()` instead of `as` casts for user-provided data.
    #[error("[VELES-023] Numeric overflow: {0}")]
    Overflow(String),

    /// Column store error (VELES-024).
    ///
    /// Indicates a column store schema or primary key validation failure.
    #[error("[VELES-024] Column store error: {0}")]
    ColumnStoreError(String),

    /// GPU operation error (VELES-025).
    ///
    /// Indicates a GPU parameter validation or operation failure.
    #[error("[VELES-025] GPU error: {0}")]
    GpuError(String),

    /// Epoch mismatch (VELES-026).
    ///
    /// Indicates a stale mmap guard detected after a remap operation.
    /// This is not recoverable — the guard must be re-acquired.
    #[error("[VELES-026] Epoch mismatch: {0}")]
    EpochMismatch(String),

    /// Guard-rail violation (VELES-027).
    ///
    /// A request crossed a configured limit. `kind` says which one, and so
    /// whether the failure is the caller's or the engine's
    /// ([`GuardRailKind::is_caller_error`]).
    #[error("[VELES-027] Guard-rail violation: {message}")]
    GuardRail {
        /// The limit the request crossed.
        kind: GuardRailKind,
        /// Human-readable description of the violation.
        message: String,
    },

    /// Invalid quantizer configuration (VELES-028).
    ///
    /// Indicates invalid parameters passed to a quantizer (e.g., empty training set,
    /// zero subspaces, dimension not divisible by subspaces).
    #[error("[VELES-028] Invalid quantizer config: {0}")]
    InvalidQuantizerConfig(String),

    /// Training failed (VELES-029).
    ///
    /// Indicates a quantizer training operation failed (convergence, insufficient
    /// data, etc.).
    #[error("[VELES-029] Training failed: {0}")]
    TrainingFailed(String),

    /// Sparse index error (VELES-030).
    #[error("[VELES-030] Sparse index error: {0}")]
    SparseIndexError(String),

    /// Database already locked by another process (VELES-031).
    #[error("[VELES-031] Database is already opened by another process: {0}")]
    DatabaseLocked(String),

    /// Invalid dimension (VELES-032).
    ///
    /// Indicates a vector dimension outside the valid range.
    #[error("[VELES-032] Invalid dimension {dimension}: must be between {min} and {max}")]
    InvalidDimension {
        /// The invalid dimension provided.
        dimension: usize,
        /// Minimum valid dimension.
        min: usize,
        /// Maximum valid dimension.
        max: usize,
    },

    /// Allocation failed (VELES-033).
    ///
    /// Indicates a memory allocation failure (out of memory or invalid layout).
    #[error("[VELES-033] Allocation failed: {0}")]
    AllocationFailed(String),

    /// Invalid collection name (VELES-034).
    ///
    /// The collection name contains forbidden characters, path separators,
    /// or is otherwise unsafe for use as a filesystem directory name.
    #[error("[VELES-034] Invalid collection name '{name}': {reason}")]
    InvalidCollectionName {
        /// The rejected name.
        name: String,
        /// Human-readable explanation of why it was rejected.
        reason: String,
    },

    /// CSR snapshot build failed (VELES-035).
    ///
    /// Indicates that building a CSR snapshot from the edge store failed
    /// (e.g., allocation failure during rebuild).
    #[error("[VELES-035] Snapshot build failed: {0}")]
    SnapshotBuildFailed(String),

    /// Incompatible schema version (VELES-036).
    ///
    /// The collection was created with a newer `VelesDB` version that uses a
    /// schema format this version cannot read. Upgrade `VelesDB` to open it.
    #[error(
        "[VELES-036] Collection created with newer schema version (v{found}), \
         current VelesDB supports up to v{supported}"
    )]
    IncompatibleSchemaVersion {
        /// The schema version found in `config.json`.
        found: u32,
        /// The maximum schema version this binary supports.
        supported: u32,
    },

    /// Access denied by the database's observer policy (VELES-037).
    ///
    /// The variant an observer refuses a read, a DDL statement or a DML
    /// mutation with (`AccessDecision::Deny`, `on_ddl_request`,
    /// `on_dml_mutation_request`), so every surface can tell "not allowed"
    /// from "malformed": REST answers `403 Forbidden`, Python raises
    /// `PermissionError`. A caller error: retrying it unchanged is refused
    /// again, and it never counts toward a circuit breaker.
    #[error("[VELES-037] Access denied: {0}")]
    AccessDenied(String),

    /// Query parameter error (VELES-038).
    ///
    /// A `$name` the statement references is not bound in the caller's
    /// parameters, or is bound to a value its position cannot take: an array
    /// or object where a scalar goes, anything but an array of numbers where
    /// a vector goes, a malformed sparse-vector map. It has its own code,
    /// apart from [`Error::Query`], so every surface can tell "supply this
    /// parameter" from "rewrite this statement": REST answers `400` with
    /// `VELES-038` (`422` `VELESQL_PARAMETER_ERROR` on the `VelesQL`
    /// endpoints), Python raises `VelesQLParameterError`, a `ValueError`. A
    /// caller error: it never counts toward a circuit breaker.
    #[error("[VELES-038] Query parameter ${name}: {reason}")]
    QueryParameter {
        /// The parameter's name, without the leading `$`.
        name: String,
        /// Why it cannot be bound: unbound, or the value it should have.
        reason: String,
    },
}

impl Error {
    /// [`Error::QueryParameter`] for a `$name` the caller bound nothing to.
    /// Another engine binds through `velesql`'s binders, which raise it.
    #[must_use]
    pub(crate) fn unbound_parameter(name: &str) -> Self {
        Self::QueryParameter {
            name: name.to_string(),
            reason: "not bound".to_string(),
        }
    }

    /// [`Error::QueryParameter`] for a `$name` bound to a value its position
    /// cannot take; `reason` says what it should be.
    pub(crate) fn invalid_parameter(name: &str, reason: impl Into<String>) -> Self {
        Self::QueryParameter {
            name: name.to_string(),
            reason: reason.into(),
        }
    }

    /// [`Error::Config`] for a sparse index named `index_name` that does not
    /// exist — `<default>` naming the unnamed one. The one wording of this
    /// refusal: core's sparse search and hybrid paths and the WASM executor
    /// raise it.
    #[must_use]
    pub fn sparse_index_not_found(index_name: &str) -> Self {
        let shown = if index_name.is_empty() {
            "<default>"
        } else {
            index_name
        };
        Self::Config(format!("Sparse index '{shown}' not found"))
    }

    /// [`Error::Query`] for a JOIN on `table` that names no join key: neither
    /// an `ON` condition nor `USING(single_column)`. The one wording of this
    /// refusal: core's column-store and hash joins and the WASM executor
    /// raise it.
    #[must_use]
    pub fn join_without_key(table: &str) -> Self {
        Self::Query(format!(
            "JOIN on table '{table}' must use ON condition or USING(single_column)."
        ))
    }

    /// [`Error::Query`] for a MATCH pattern without a node. The one wording
    /// of this refusal: core's walk and the WASM executor raise it.
    #[must_use]
    pub fn pattern_without_node() -> Self {
        Self::Query("Pattern must have at least one node".to_string())
    }

    /// Returns the error code (e.g., "VELES-001").
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::CollectionExists(_) => "VELES-001",
            Self::CollectionNotFound(_) => "VELES-002",
            Self::PointNotFound(_) => "VELES-003",
            Self::DimensionMismatch { .. } => "VELES-004",
            Self::InvalidVector(_) => "VELES-005",
            Self::Storage(_) => "VELES-006",
            Self::Index(_) => "VELES-007",
            Self::IndexCorrupted(_) => "VELES-008",
            Self::Config(_) => "VELES-009",
            Self::Query(_) => "VELES-010",
            Self::Io(_) => "VELES-011",
            Self::Serialization(_) => "VELES-012",
            Self::Internal(_) => "VELES-013",
            Self::VectorNotAllowed(_) => "VELES-014",
            Self::SearchNotSupported(_) => "VELES-015",
            Self::VectorRequired(_) => "VELES-016",
            Self::SchemaValidation(_) => "VELES-017",
            Self::GraphNotSupported(_) => "VELES-018",
            Self::EdgeExists(_) => "VELES-019",
            Self::EdgeNotFound(_) => "VELES-020",
            Self::InvalidEdgeLabel(_) => "VELES-021",
            Self::NodeNotFound(_) => "VELES-022",
            Self::Overflow(_) => "VELES-023",
            Self::ColumnStoreError(_) => "VELES-024",
            Self::GpuError(_) => "VELES-025",
            Self::EpochMismatch(_) => "VELES-026",
            Self::GuardRail { .. } => "VELES-027",
            Self::InvalidQuantizerConfig(_) => "VELES-028",
            Self::TrainingFailed(_) => "VELES-029",
            Self::SparseIndexError(_) => "VELES-030",
            Self::DatabaseLocked(_) => "VELES-031",
            Self::InvalidDimension { .. } => "VELES-032",
            Self::AllocationFailed(_) => "VELES-033",
            Self::InvalidCollectionName { .. } => "VELES-034",
            Self::SnapshotBuildFailed(_) => "VELES-035",
            Self::IncompatibleSchemaVersion { .. } => "VELES-036",
            Self::AccessDenied(_) => "VELES-037",
            Self::QueryParameter { .. } => "VELES-038",
        }
    }

    /// Returns true if this error is recoverable.
    ///
    /// Non-recoverable errors include corruption and internal errors.
    #[must_use]
    pub const fn is_recoverable(&self) -> bool {
        !matches!(
            self,
            Self::IndexCorrupted(_)
                | Self::Internal(_)
                | Self::EpochMismatch(_)
                | Self::AllocationFailed(_)
                | Self::IncompatibleSchemaVersion { .. }
        )
    }

    /// Returns true if the caller caused this error, not the engine.
    ///
    /// A caller error — a malformed or unsupported query, a missing bind
    /// parameter, a vector of the wrong dimension, a name that does not
    /// exist or already does — is answered the same way however healthy the
    /// collection is, so it says nothing about that health: retrying it
    /// unchanged fails again. A guard-rail stop is classified by its
    /// [`GuardRailKind`]: a limit the request itself crossed (the client's
    /// own timeout budget, a depth, cardinality or capacity ceiling, the
    /// client's rate limit) is the caller's. Every other error comes from the
    /// engine or its environment (storage, I/O, corruption, an internal
    /// fault, a timeout under the server's own limit, the memory budget): load
    /// the engine could not carry, which is what the circuit breaker sheds.
    ///
    /// This is the one classification of that question. The circuit breaker
    /// counts only errors for which it is `false`
    /// (`CircuitBreaker::record_error`), and the server answers a caller error
    /// with a `4xx` status, every other error with a `5xx`.
    ///
    /// The matches are exhaustive on purpose: a new variant or kind does not
    /// compile until someone decides which side it is on.
    #[must_use]
    pub const fn is_caller_error(&self) -> bool {
        match self {
            Self::CollectionExists(_)
            | Self::CollectionNotFound(_)
            | Self::PointNotFound(_)
            | Self::DimensionMismatch { .. }
            | Self::InvalidVector(_)
            | Self::Config(_)
            | Self::Query(_)
            | Self::VectorNotAllowed(_)
            | Self::SearchNotSupported(_)
            | Self::VectorRequired(_)
            | Self::SchemaValidation(_)
            | Self::GraphNotSupported(_)
            | Self::EdgeExists(_)
            | Self::EdgeNotFound(_)
            | Self::InvalidEdgeLabel(_)
            | Self::NodeNotFound(_)
            | Self::Overflow(_)
            | Self::InvalidQuantizerConfig(_)
            | Self::InvalidDimension { .. }
            | Self::InvalidCollectionName { .. }
            | Self::AccessDenied(_)
            | Self::QueryParameter { .. } => true,
            Self::GuardRail { kind, .. } => kind.is_caller_error(),
            Self::Storage(_)
            | Self::Index(_)
            | Self::IndexCorrupted(_)
            | Self::Io(_)
            | Self::Serialization(_)
            | Self::Internal(_)
            | Self::ColumnStoreError(_)
            | Self::GpuError(_)
            | Self::EpochMismatch(_)
            | Self::TrainingFailed(_)
            | Self::SparseIndexError(_)
            | Self::DatabaseLocked(_)
            | Self::AllocationFailed(_)
            | Self::SnapshotBuildFailed(_)
            | Self::IncompatibleSchemaVersion { .. } => false,
        }
    }
}

/// Conversion from `VelesQL` `ParseError`.
impl From<crate::velesql::ParseError> for Error {
    fn from(err: crate::velesql::ParseError) -> Self {
        Self::Query(err.to_string())
    }
}

/// Conversion from a semantic validation failure (`QueryValidator`): a query
/// error whose message carries the validation code (`[V012] …`).
impl From<crate::velesql::ValidationError> for Error {
    fn from(err: crate::velesql::ValidationError) -> Self {
        Self::Query(err.to_string())
    }
}

/// Conversion from a fusion failure (weights, directions): a configuration
/// error, the caller's fusion settings being what is wrong.
impl From<crate::fusion::FusionError> for Error {
    fn from(err: crate::fusion::FusionError) -> Self {
        Self::Config(format!("Fusion error: {err}"))
    }
}

/// Conversion from `GuardRailViolation` — surfaces limit violations as
/// [`Error::GuardRail`], keeping which limit was crossed.
impl From<crate::guardrails::GuardRailViolation> for Error {
    fn from(v: crate::guardrails::GuardRailViolation) -> Self {
        Self::GuardRail {
            kind: v.kind(),
            message: v.to_string(),
        }
    }
}

/// The limit a request crossed, carried by [`Error::GuardRail`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum GuardRailKind {
    /// The query ran past its time budget.
    Timeout(TimeoutBudget),
    /// A graph traversal went deeper than `max_depth`.
    Depth,
    /// A query produced more intermediate results than `max_cardinality`.
    Cardinality,
    /// A query used more memory than `memory_limit_bytes`.
    Memory,
    /// A request would cross a configured capacity: `max_collections`,
    /// `max_dimensions`, `max_vectors_per_collection`, `max_payload_size` or
    /// `max_perfect_mode_vectors`.
    Capacity,
    /// The client sent more queries per second than `rate_limit_qps`.
    RateLimit,
    /// The collection's circuit breaker is open and refused the request.
    CircuitOpen,
}

/// Whose time budget a [`GuardRailKind::Timeout`] ran out of.
///
/// One classification decides it for every surface:
/// `QueryLimits::timeout_budget`, shared by `VelesQL` `WITH (timeout_ms = N)`
/// and the REST search's `timeout_ms`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TimeoutBudget {
    /// A budget the request chose strictly tighter than the server's limit
    /// (`QueryLimits::timeout_ms`), or chose with no server limit set.
    /// Running out of it says the client asked for too little time, not that
    /// the engine is slow.
    Client,
    /// The server's limit, or a request budget at or above it: a query that
    /// runs that long is load the engine could not carry.
    Configured,
}

impl GuardRailKind {
    /// Returns true if crossing this limit is the caller's error rather than
    /// a sign of an unhealthy engine (see [`Error::is_caller_error`]).
    ///
    /// The client's own timeout budget, a depth, cardinality or capacity
    /// ceiling its request crossed, and its rate limit are the caller's: the
    /// same request is refused however healthy the collection is. A timeout
    /// under the server's limit and the memory budget are load. An open
    /// circuit breaker is the breaker's own answer, neither.
    #[must_use]
    pub const fn is_caller_error(self) -> bool {
        match self {
            Self::Timeout(TimeoutBudget::Client)
            | Self::Depth
            | Self::Cardinality
            | Self::Capacity
            | Self::RateLimit => true,
            Self::Timeout(TimeoutBudget::Configured) | Self::Memory | Self::CircuitOpen => false,
        }
    }
}
