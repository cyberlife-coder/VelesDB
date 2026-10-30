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

/// Code of [`Error::TaskJoin`], the one plugin condition core has no code for.
const TASK_JOIN_CODE: &str = "TASK_JOIN";

impl Error {
    /// The code and recoverability this error reports, both read from core's
    /// taxonomy: a core error answers for itself, and a plugin-local variant
    /// answers through the core variant naming the same condition, so neither
    /// is a copy that can drift from [`velesdb_core::Error::code`].
    fn classification(&self) -> (&'static str, bool) {
        use velesdb_core::Error as Core;
        let equivalent = match self {
            Self::Database(core_err) => return (core_err.code(), core_err.is_recoverable()),
            // A blocking task that panicked or was cancelled at shutdown:
            // retrying the same command cannot help.
            Self::TaskJoin(_) => return (TASK_JOIN_CODE, false),
            Self::CollectionNotFound(_) => Core::CollectionNotFound(String::new()),
            // Agent memory entries are points, so a missing one is VELES-003.
            Self::NotFound(_) => Core::PointNotFound(0),
            Self::DimensionMismatch { expected, actual } => Core::DimensionMismatch {
                expected: *expected,
                actual: *actual,
            },
            Self::InvalidConfig(_) | Self::ConfigLoad { .. } => Core::Config(String::new()),
            Self::Serialization(_) => Core::Serialization(String::new()),
            Self::Io(io_err) => Core::Io(io_err.kind().into()),
        };
        (equivalent.code(), equivalent.is_recoverable())
    }
}

/// Serializable error for Tauri commands.
#[derive(Debug, Serialize, Deserialize)]
pub struct CommandError {
    /// Error message.
    pub message: String,
    /// Core taxonomy code (`VELES-###`) for programmatic handling, or
    /// `TASK_JOIN` for a blocking task that could not be joined.
    pub code: String,
    /// Whether retrying can succeed, as [`velesdb_core::Error::is_recoverable`]
    /// answers for the same condition.
    pub recoverable: bool,
}

impl From<Error> for CommandError {
    fn from(err: Error) -> Self {
        let (code, recoverable) = err.classification();
        Self {
            message: err.to_string(),
            code: code.to_string(),
            recoverable,
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
