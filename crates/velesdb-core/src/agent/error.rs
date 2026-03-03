//! Error types for `AgentMemory` operations.

use super::snapshot::SnapshotError;
use thiserror::Error;

/// Error variants returned by agent memory operations.
#[derive(Debug, Error)]
pub enum AgentMemoryError {
    /// Returned when a memory subsystem collection cannot be created or opened.
    #[error("Failed to initialize memory: {0}")]
    InitializationError(String),

    /// Returned when an underlying collection operation fails.
    #[error("Collection error: {0}")]
    CollectionError(String),

    /// Returned when a requested memory entry does not exist.
    #[error("Item not found: {0}")]
    NotFound(String),

    /// Returned when a provided embedding dimension does not match the stored dimension.
    #[error("Invalid embedding dimension: expected {expected}, got {actual}")]
    DimensionMismatch {
        /// Expected embedding dimension.
        expected: usize,
        /// Actual embedding dimension provided.
        actual: usize,
    },

    /// Returned when a core database error propagates from the storage layer.
    #[error("Database error: {0}")]
    DatabaseError(#[from] crate::error::Error),

    /// Returned when a snapshot operation fails.
    #[error("Snapshot error: {0}")]
    SnapshotError(String),

    /// Returned when an IO error occurs during snapshot persistence.
    #[error("IO error: {0}")]
    IoError(String),
}

impl From<SnapshotError> for AgentMemoryError {
    fn from(e: SnapshotError) -> Self {
        Self::SnapshotError(e.to_string())
    }
}

impl From<std::io::Error> for AgentMemoryError {
    fn from(e: std::io::Error) -> Self {
        Self::IoError(e.to_string())
    }
}
