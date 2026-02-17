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

    /// Vector not allowed on metadata-only collection (VELES-014).
    #[error("[VELES-014] Vector not allowed on metadata-only collection '{0}'")]
    VectorNotAllowed(String),

    /// Search not supported on metadata-only collection (VELES-015).
    #[error("[VELES-015] Vector search not supported on metadata-only collection '{0}'. Use query() instead.")]
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

    /// Unsupported feature (VELES-027).
    ///
    /// Indicates a query uses a feature that is parsed but not yet implemented.
    #[error("[VELES-027] Unsupported feature: {0}")]
    UnsupportedFeature(String),

    /// Node already exists (VELES-028).
    #[error("[VELES-028] Node with ID '{0}' already exists")]
    NodeExists(u64),
}

impl Error {
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
            Self::UnsupportedFeature(_) => "VELES-027",
            Self::NodeExists(_) => "VELES-028",
        }
    }

    /// Returns true if this error is recoverable.
    ///
    /// Non-recoverable errors include corruption and internal errors.
    #[must_use]
    pub const fn is_recoverable(&self) -> bool {
        !matches!(
            self,
            Self::IndexCorrupted(_) | Self::Internal(_) | Self::EpochMismatch(_)
        )
    }
}

/// Conversion from `VelesQL` `ParseError`.
impl From<crate::velesql::ParseError> for Error {
    fn from(err: crate::velesql::ParseError) -> Self {
        Self::Query(err.to_string())
    }
}
