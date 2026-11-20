//! Tests for `error` module

use super::error::*;

// -------------------------------------------------------------------------
// Error code tests
// -------------------------------------------------------------------------

/// One error of every variant, each at the index [`variant_index`] gives it.
fn every_variant() -> Vec<Error> {
    vec![
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
        Error::GuardRail {
            kind: GuardRailKind::Depth,
            message: "test".into(),
        },
        Error::InvalidQuantizerConfig("test".into()),
        Error::TrainingFailed("test".into()),
        Error::SparseIndexError("test".into()),
        Error::DatabaseLocked("test".into()),
        Error::InvalidDimension {
            dimension: 0,
            min: 1,
            max: 65_536,
        },
        Error::AllocationFailed("test".into()),
        Error::InvalidCollectionName {
            name: "test".into(),
            reason: "test".into(),
        },
        Error::SnapshotBuildFailed("test".into()),
        Error::IncompatibleSchemaVersion {
            found: 9,
            supported: 1,
        },
        Error::AccessDenied("test".into()),
        Error::QueryParameter {
            name: "v".into(),
            reason: "not bound".into(),
        },
    ]
}

/// The position of `err`'s variant in [`every_variant`]. The match has no
/// wildcard arm, so a new variant does not compile until it is given an
/// index here — and then fails [`test_every_variant_is_listed`] until
/// [`every_variant`] lists it.
fn variant_index(err: &Error) -> usize {
    match err {
        Error::CollectionExists(_) => 0,
        Error::CollectionNotFound(_) => 1,
        Error::PointNotFound(_) => 2,
        Error::DimensionMismatch { .. } => 3,
        Error::InvalidVector(_) => 4,
        Error::Storage(_) => 5,
        Error::Index(_) => 6,
        Error::IndexCorrupted(_) => 7,
        Error::Config(_) => 8,
        Error::Query(_) => 9,
        Error::Io(_) => 10,
        Error::Serialization(_) => 11,
        Error::Internal(_) => 12,
        Error::VectorNotAllowed(_) => 13,
        Error::SearchNotSupported(_) => 14,
        Error::VectorRequired(_) => 15,
        Error::SchemaValidation(_) => 16,
        Error::GraphNotSupported(_) => 17,
        Error::EdgeExists(_) => 18,
        Error::EdgeNotFound(_) => 19,
        Error::InvalidEdgeLabel(_) => 20,
        Error::NodeNotFound(_) => 21,
        Error::Overflow(_) => 22,
        Error::ColumnStoreError(_) => 23,
        Error::GpuError(_) => 24,
        Error::EpochMismatch(_) => 25,
        Error::GuardRail { .. } => 26,
        Error::InvalidQuantizerConfig(_) => 27,
        Error::TrainingFailed(_) => 28,
        Error::SparseIndexError(_) => 29,
        Error::DatabaseLocked(_) => 30,
        Error::InvalidDimension { .. } => 31,
        Error::AllocationFailed(_) => 32,
        Error::InvalidCollectionName { .. } => 33,
        Error::SnapshotBuildFailed(_) => 34,
        Error::IncompatibleSchemaVersion { .. } => 35,
        Error::AccessDenied(_) => 36,
        Error::QueryParameter { .. } => 37,
    }
}

#[test]
fn test_every_variant_is_listed() {
    let indices: Vec<usize> = every_variant().iter().map(variant_index).collect();
    let expected: Vec<usize> = (0..indices.len()).collect();
    assert_eq!(
        indices, expected,
        "every_variant lists each variant once, in order"
    );
    assert_eq!(indices.len(), 38, "a variant is missing from every_variant");
}

#[test]
fn test_error_codes_are_unique() {
    let codes: Vec<&str> = every_variant().iter().map(Error::code).collect();

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

/// Every variant's message leads with its code in brackets and a space —
/// `[VELES-038] Query parameter $v: not bound` — the form every surface
/// that prints a core error shows (`ERROR_CODES.md`).
#[test]
fn test_every_message_leads_with_its_code() {
    for err in every_variant() {
        let message = err.to_string();
        let prefix = format!("[{}] ", err.code());
        assert!(
            message.starts_with(&prefix),
            "{message:?} must start with {prefix:?}"
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

#[test]
fn test_from_fusion_error_is_a_configuration_error() {
    let fusion = crate::fusion::FusionError::DirectionCountMismatch {
        directions: 1,
        branches: 2,
    };
    let expected = format!("[VELES-009] Configuration error: Fusion error: {fusion}");

    let err = Error::from(fusion);

    assert_eq!(err.code(), "VELES-009");
    assert_eq!(err.to_string(), expected);
}

#[test]
fn test_from_validation_error_is_a_query_error_carrying_its_code() {
    let query = crate::velesql::Parser::parse(
        "SELECT * FROM docs WHERE similarity(vector, $v) > 0.5 OR similarity(vector, $w) > 0.5",
    )
    .expect("test: parses");
    let validation =
        crate::velesql::QueryValidator::validate(&query).expect_err("test: V-rule refusal");
    let expected = format!("[VELES-010] Query error: {validation}");

    let err = Error::from(validation);

    assert_eq!(err.code(), "VELES-010");
    assert_eq!(err.to_string(), expected);
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

// -------------------------------------------------------------------------
// TrainingFailed error tests (VELES-029, PQ-06)
// -------------------------------------------------------------------------

#[test]
fn test_training_failed_code() {
    let err = Error::TrainingFailed("convergence failure".into());
    assert_eq!(err.code(), "VELES-029");
}

#[test]
fn test_training_failed_display() {
    let err = Error::TrainingFailed("insufficient data for 256 clusters".into());
    let display = format!("{err}");
    assert!(display.contains("[VELES-029]"));
    assert!(display.contains("Training failed"));
    assert!(display.contains("insufficient data for 256 clusters"));
}

#[test]
fn test_training_failed_is_recoverable() {
    // Training failure is recoverable (user can add more data or change params)
    assert!(Error::TrainingFailed("not enough vectors".into()).is_recoverable());
}

// -------------------------------------------------------------------------
// Caller-error classification (what the circuit breaker may count)
// -------------------------------------------------------------------------

#[test]
fn test_caller_errors_are_classified_as_the_callers() {
    let caller = [
        Error::CollectionNotFound("x".into()),
        Error::CollectionExists("x".into()),
        Error::DimensionMismatch {
            expected: 4,
            actual: 2,
        },
        Error::Query("[V015] unsupported MATCH ORDER BY".into()),
        Error::GraphNotSupported("MATCH ORDER BY expression 'd'".into()),
        Error::Config("bad option".into()),
        Error::NodeNotFound(7),
        Error::GuardRail {
            kind: GuardRailKind::Timeout(TimeoutBudget::Client),
            message: "Query timed out".into(),
        },
        Error::GuardRail {
            kind: GuardRailKind::Cardinality,
            message: "Cardinality exceeded".into(),
        },
        Error::GuardRail {
            kind: GuardRailKind::Depth,
            message: "Traversal depth exceeded".into(),
        },
        Error::GuardRail {
            kind: GuardRailKind::Capacity,
            message: "max_collections limit reached".into(),
        },
        Error::GuardRail {
            kind: GuardRailKind::RateLimit,
            message: "Rate limit exceeded".into(),
        },
    ];
    for err in &caller {
        assert!(err.is_caller_error(), "{err} must be the caller's error");
    }
}

#[test]
fn test_engine_failures_and_load_guard_rails_are_not_caller_errors() {
    let engine = [
        Error::Io(std::io::Error::other("disk gone")),
        Error::Storage("short write".into()),
        Error::IndexCorrupted("checksum mismatch".into()),
        Error::Internal("unexpected state".into()),
        Error::AllocationFailed("oom".into()),
        Error::GuardRail {
            kind: GuardRailKind::Timeout(TimeoutBudget::Configured),
            message: "Query timed out".into(),
        },
        Error::GuardRail {
            kind: GuardRailKind::Memory,
            message: "Memory limit exceeded".into(),
        },
        Error::GuardRail {
            kind: GuardRailKind::CircuitOpen,
            message: "Circuit breaker open".into(),
        },
    ];
    for err in &engine {
        assert!(!err.is_caller_error(), "{err} must count as the engine's");
    }
}

#[test]
fn test_access_denied_is_its_own_caller_error() {
    // An observer's refusal is not a malformed query (VELES-010): it has its
    // own code, so a surface can answer "forbidden", and it never counts
    // toward a circuit breaker.
    let err = Error::AccessDenied("read denied by policy".into());
    assert_eq!(err.code(), "VELES-037");
    assert!(err.is_caller_error());
    assert!(err.is_recoverable());
    assert!(err.to_string().starts_with("[VELES-037] Access denied"));
}

#[test]
fn test_query_parameter_is_its_own_caller_error() {
    // A `$name` the caller did not bind, or bound to a value its position
    // cannot take, is not a malformed statement (VELES-010): it has its own
    // code, so a surface can tell "supply the parameter" from "rewrite the
    // query", and it never counts toward a circuit breaker.
    let err = Error::QueryParameter {
        name: "v".into(),
        reason: "not bound".into(),
    };
    assert_eq!(err.code(), "VELES-038");
    assert!(err.is_caller_error());
    assert!(err.is_recoverable());
    assert_eq!(err.to_string(), "[VELES-038] Query parameter $v: not bound");
}

// -------------------------------------------------------------------------
// Shared refusal constructors
// -------------------------------------------------------------------------

#[test]
fn sparse_index_not_found_with_named_index() {
    let err = Error::sparse_index_not_found("my_index");
    let msg = err.to_string();
    assert!(
        msg.contains("my_index"),
        "Error message should contain the index name, got: {msg}"
    );
    // Verify it is an Error::Config variant.
    assert!(matches!(err, Error::Config(_)));
}

#[test]
fn sparse_index_not_found_empty_name_shows_default() {
    let err = Error::sparse_index_not_found("");
    let msg = err.to_string();
    assert!(
        msg.contains("<default>"),
        "Empty name should display <default>, got: {msg}"
    );
    assert!(matches!(err, Error::Config(_)));
}

#[test]
fn sparse_index_not_found_nonempty_does_not_show_default() {
    let err = Error::sparse_index_not_found("title_sparse");
    let msg = err.to_string();
    assert!(
        !msg.contains("<default>"),
        "Non-empty name should not show <default>, got: {msg}"
    );
}

#[test]
fn join_without_key_names_the_table_and_the_two_accepted_forms() {
    let err = Error::join_without_key("orders");
    assert_eq!(err.code(), "VELES-010");
    assert_eq!(
        err.to_string(),
        "[VELES-010] Query error: JOIN on table 'orders' must use ON condition or \
         USING(single_column)."
    );
}
