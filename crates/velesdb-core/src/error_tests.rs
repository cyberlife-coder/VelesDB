//! Tests for `error` module

use super::error::*;

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
        Error::VectorNotAllowed("test".into()),
        Error::SearchNotSupported("test".into()),
        Error::VectorRequired("test".into()),
        Error::SchemaValidation("test".into()),
        Error::GraphNotSupported("test".into()),
        Error::EdgeExists(1),
        Error::EdgeNotFound(1),
        Error::InvalidEdgeLabel("test".into()),
        Error::NodeNotFound(1),
        Error::Overflow("test".into()),
        Error::ColumnStoreError("test".into()),
        Error::GpuError("test".into()),
        Error::EpochMismatch("test".into()),
        Error::UnsupportedFeature("test".into()),
        Error::NodeExists(1),
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
