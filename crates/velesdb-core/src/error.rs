//! Error types for `VelesDB`.

use thiserror::Error;

/// Result type alias for `VelesDB` operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors that can occur in `VelesDB` operations.
#[derive(Error, Debug)]
pub enum Error {
    /// Collection already exists.
    #[error("Collection '{0}' already exists")]
    CollectionExists(String),

    /// Collection not found.
    #[error("Collection '{0}' not found")]
    CollectionNotFound(String),

    /// Point not found.
    #[error("Point with ID '{0}' not found")]
    PointNotFound(u64),

    /// Dimension mismatch.
    #[error("Vector dimension mismatch: expected {expected}, got {actual}")]
    DimensionMismatch {
        /// Expected dimension.
        expected: usize,
        /// Actual dimension.
        actual: usize,
    },

    /// Invalid vector.
    #[error("Invalid vector: {0}")]
    InvalidVector(String),

    /// Storage error.
    #[error("Storage error: {0}")]
    Storage(String),

    /// Index error.
    #[error("Index error: {0}")]
    Index(String),

    /// IO error.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// Serialization error.
    #[error("Serialization error: {0}")]
    Serialization(String),

    /// Internal error.
    #[error("Internal error: {0}")]
    Internal(String),
}
