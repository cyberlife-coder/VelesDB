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
        }
    }

    /// Returns true if this error is recoverable.
    ///
    /// Non-recoverable errors include corruption and internal errors.
    #[must_use]
    pub const fn is_recoverable(&self) -> bool {
        !matches!(self, Self::IndexCorrupted(_) | Self::Internal(_))
    }
}

/// Conversion from `VelesQL` `ParseError`.
impl From<crate::velesql::ParseError> for Error {
    fn from(err: crate::velesql::ParseError) -> Self {
        Self::Query(err.to_string())
    }
}

// ============================================================================
// TDD TESTS - Written BEFORE implementation changes
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -------------------------------------------------------------------------
    // Error code tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_error_codes_are_unique() {
        // Arrange - create all error variants
        let errors: Vec<Error> = vec![
            Error::CollectionExists("test".into()),
            Error::CollectionNotFound("test".into()),
            Error::PointNotFound(1),
            Error::DimensionMismatch {
                expected: 768,
                actual: 512,
            },
            Error::InvalidVector("test".into()),
            Error::Storage("test".into()),
            Error::Index("test".into()),
            Error::IndexCorrupted("test".into()),
            Error::Config("test".into()),
            Error::Query("test".into()),
            Error::Io(std::io::Error::other("test")),
            Error::Serialization("test".into()),
            Error::Internal("test".into()),
        ];

        // Act - collect all codes
        let codes: Vec<&str> = errors.iter().map(Error::code).collect();

        // Assert - all codes are unique and follow pattern
        let mut unique_codes = codes.clone();
        unique_codes.sort_unstable();
        unique_codes.dedup();
        assert_eq!(
            codes.len(),
            unique_codes.len(),
            "Error codes must be unique"
        );

        for code in &codes {
            assert!(
                code.starts_with("VELES-"),
                "Code {code} should start with VELES-"
            );
        }
    }

    #[test]
    fn test_error_display_includes_code() {
        // Arrange
        let err = Error::CollectionNotFound("documents".into());

        // Act
        let display = format!("{err}");

        // Assert
        assert!(display.contains("VELES-002"));
        assert!(display.contains("documents"));
    }

    #[test]
    fn test_dimension_mismatch_display() {
        // Arrange
        let err = Error::DimensionMismatch {
            expected: 768,
            actual: 512,
        };

        // Act
        let display = format!("{err}");

        // Assert
        assert!(display.contains("768"));
        assert!(display.contains("512"));
        assert!(display.contains("VELES-004"));
    }

    // -------------------------------------------------------------------------
    // Conversion tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_from_io_error() {
        // Arrange
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");

        // Act
        let err: Error = io_err.into();

        // Assert
        assert_eq!(err.code(), "VELES-011");
        assert!(format!("{err}").contains("file not found"));
    }

    #[test]
    fn test_from_parse_error() {
        // Arrange
        let parse_err = crate::velesql::ParseError::syntax(15, "FORM", "Expected FROM");

        // Act
        let err: Error = parse_err.into();

        // Assert
        assert_eq!(err.code(), "VELES-010");
        assert!(format!("{err}").contains("FROM"));
    }

    // -------------------------------------------------------------------------
    // Recoverable tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_recoverable_errors() {
        // These errors are recoverable (user can fix and retry)
        assert!(Error::CollectionNotFound("x".into()).is_recoverable());
        assert!(Error::DimensionMismatch {
            expected: 768,
            actual: 512
        }
        .is_recoverable());
        assert!(Error::Query("syntax error".into()).is_recoverable());
    }

    #[test]
    fn test_non_recoverable_errors() {
        // These errors indicate serious problems
        assert!(!Error::IndexCorrupted("checksum mismatch".into()).is_recoverable());
        assert!(!Error::Internal("unexpected state".into()).is_recoverable());
    }

    // -------------------------------------------------------------------------
    // Professional API tests (for Python/Node exposure)
    // -------------------------------------------------------------------------

    #[test]
    fn test_error_is_send_sync() {
        // Required for async/threaded contexts
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Error>();
    }

    #[test]
    fn test_error_debug_impl() {
        // Debug should be available for logging
        let err = Error::Storage("disk full".into());
        let debug = format!("{err:?}");
        assert!(debug.contains("Storage"));
        assert!(debug.contains("disk full"));
    }
}
