//! Error types for AgentMemory operations.

use super::snapshot::SnapshotError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AgentMemoryError {
    #[error("Failed to initialize memory: {0}")]
    InitializationError(String),

    #[error("Collection error: {0}")]
    CollectionError(String),

    #[error("Item not found: {0}")]
    NotFound(String),

    #[error("Invalid embedding dimension: expected {expected}, got {actual}")]
    DimensionMismatch { expected: usize, actual: usize },

    #[error("Database error: {0}")]
    DatabaseError(#[from] crate::error::Error),

    #[error("Snapshot error: {0}")]
    SnapshotError(String),

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
