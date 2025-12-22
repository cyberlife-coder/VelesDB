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

    /// Invalid configuration.
    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),

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
            Error::Database(_) => "DATABASE_ERROR",
            Error::CollectionNotFound(_) => "COLLECTION_NOT_FOUND",
            Error::InvalidConfig(_) => "INVALID_CONFIG",
            Error::Serialization(_) => "SERIALIZATION_ERROR",
            Error::Io(_) => "IO_ERROR",
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

/// Result type alias for plugin operations.
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display_collection_not_found() {
        // Arrange
        let err = Error::CollectionNotFound("test_collection".to_string());

        // Act
        let message = err.to_string();

        // Assert
        assert_eq!(message, "Collection 'test_collection' not found");
    }

    #[test]
    fn test_error_display_invalid_config() {
        // Arrange
        let err = Error::InvalidConfig("missing dimension".to_string());

        // Act
        let message = err.to_string();

        // Assert
        assert_eq!(message, "Invalid configuration: missing dimension");
    }

    #[test]
    fn test_command_error_from_error() {
        // Arrange
        let err = Error::CollectionNotFound("docs".to_string());

        // Act
        let cmd_err: CommandError = err.into();

        // Assert
        assert_eq!(cmd_err.code, "COLLECTION_NOT_FOUND");
        assert!(cmd_err.message.contains("docs"));
    }

    #[test]
    fn test_command_error_codes() {
        // Arrange & Act & Assert
        let cases = vec![
            (
                Error::CollectionNotFound("x".to_string()),
                "COLLECTION_NOT_FOUND",
            ),
            (Error::InvalidConfig("x".to_string()), "INVALID_CONFIG"),
            (Error::Serialization("x".to_string()), "SERIALIZATION_ERROR"),
        ];

        for (err, expected_code) in cases {
            let cmd_err: CommandError = err.into();
            assert_eq!(cmd_err.code, expected_code);
        }
    }
}
