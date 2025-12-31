//! Error types for velesdb-migrate.

use thiserror::Error;

/// Migration error types.
#[derive(Error, Debug)]
pub enum Error {
    /// Configuration error.
    #[error("Configuration error: {0}")]
    Config(String),

    /// Connection error to source database.
    #[error("Source connection error: {0}")]
    SourceConnection(String),

    /// Connection error to destination (`VelesDB`).
    #[error("Destination connection error: {0}")]
    DestinationConnection(String),

    /// Data extraction error.
    #[error("Extraction error: {0}")]
    Extraction(String),

    /// Data transformation error.
    #[error("Transformation error: {0}")]
    Transformation(String),

    /// Data loading error.
    #[error("Loading error: {0}")]
    Loading(String),

    /// Schema mismatch between source and destination.
    #[error("Schema mismatch: {0}")]
    SchemaMismatch(String),

    /// Checkpoint/resume error.
    #[error("Checkpoint error: {0}")]
    Checkpoint(String),

    /// HTTP request error.
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    /// JSON serialization/deserialization error.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// YAML parsing error.
    #[error("YAML error: {0}")]
    Yaml(#[from] serde_yaml::Error),

    /// IO error.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// `VelesDB` core error.
    #[error("VelesDB error: {0}")]
    VelesDb(String),

    /// Unsupported source type.
    #[error("Unsupported source: {0}")]
    UnsupportedSource(String),

    /// Rate limit exceeded.
    #[error("Rate limit exceeded, retry after {0} seconds")]
    RateLimit(u64),

    /// Authentication error.
    #[error("Authentication failed: {0}")]
    Authentication(String),
}

/// Result type alias for migration operations.
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = Error::Config("missing API key".to_string());
        assert_eq!(err.to_string(), "Configuration error: missing API key");
    }

    #[test]
    fn test_error_from_io() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let err: Error = io_err.into();
        assert!(matches!(err, Error::Io(_)));
    }
}
