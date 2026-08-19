//! Error types for the `VelesDB` Tauri plugin.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Plugin error type.
#[derive(Debug, Error)]
pub enum Error {
    /// Database error from velesdb-core.
    #[error("Database error: {0}")]
    Database(#[from] velesdb_core::Error),

    /// Collection not found.
    #[error("Collection '{0}' not found")]
    CollectionNotFound(String),

    /// A requested memory entry does not exist.
    #[error("Not found: {0}")]
    NotFound(String),

    /// A provided embedding dimension does not match the stored dimension.
    #[error("Invalid embedding dimension: expected {expected}, got {actual}")]
    DimensionMismatch {
        /// Expected embedding dimension.
        expected: usize,
        /// Actual embedding dimension provided.
        actual: usize,
    },

    /// Invalid configuration.
    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),

    /// An explicit engine config file could not be loaded (issue #1549).
    ///
    /// Raised by `Builder::with_config_path` when the TOML file is missing,
    /// unparsable, or fails engine validation. The typed core
    /// [`velesdb_core::config::ConfigError`] is preserved as the source so
    /// hosts can match on the exact failure.
    #[error("Failed to load VelesDB config from {path}: {source}")]
    ConfigLoad {
        /// The config file path that failed to load.
        path: String,
        /// The typed core configuration error.
        #[source]
        source: velesdb_core::config::ConfigError,
    },

    /// A blocking task could not be joined (it panicked or the async
    /// runtime is shutting down).
    #[error("Blocking task failed: {0}")]
    TaskJoin(String),

    /// Serialization error.
    #[error("Serialization error: {0}")]
    Serialization(String),

    /// IO error.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Serializable error for Tauri commands.
#[derive(Debug, Serialize, Deserialize)]
pub struct CommandError {
    /// Error message.
    pub message: String,
    /// Error code for programmatic handling.
    pub code: String,
}

impl From<Error> for CommandError {
    fn from(err: Error) -> Self {
        let code = match &err {
            Error::Database(core_err) => core_err.code(),
            Error::CollectionNotFound(_) => "VELES-002",
            Error::NotFound(_) => "NOT_FOUND",
            Error::DimensionMismatch { .. } => "DIMENSION_MISMATCH",
            Error::InvalidConfig(_) | Error::ConfigLoad { .. } => "INVALID_CONFIG",
            Error::TaskJoin(_) => "TASK_JOIN",
            Error::Serialization(_) => "SERIALIZATION_ERROR",
            Error::Io(_) => "VELES-011",
        };
        Self {
            message: err.to_string(),
            code: code.to_string(),
        }
    }
}

impl From<serde_json::Error> for Error {
    fn from(err: serde_json::Error) -> Self {
        Self::Serialization(err.to_string())
    }
}

impl From<velesdb_core::agent::AgentMemoryError> for Error {
    fn from(err: velesdb_core::agent::AgentMemoryError) -> Self {
        use velesdb_core::agent::AgentMemoryError as A;
        match err {
            A::NotFound(msg) => Self::NotFound(msg),
            A::DimensionMismatch { expected, actual } => {
                Self::DimensionMismatch { expected, actual }
            }
            A::DatabaseError(core_err) => Self::Database(core_err),
            other => Self::InvalidConfig(other.to_string()),
        }
    }
}

/// Result type alias for plugin operations.
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
#[path = "error_tests.rs"]
mod tests;
