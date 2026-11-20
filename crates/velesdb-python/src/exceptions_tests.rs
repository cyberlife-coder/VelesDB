use super::*;
use crate::collection_helpers::{core_err, core_err_with_collection};
use pyo3::{PyTypeInfo, Python};

/// DimensionMismatch maps to DimensionMismatchError, not RuntimeError.
#[test]
fn test_core_err_dimension_mismatch_type() {
    pyo3::Python::initialize();
    Python::attach(|py| {
        let err = core_err(velesdb_core::Error::DimensionMismatch {
            expected: 768,
            actual: 512,
        });
        // The exception type must be DimensionMismatchError.
        assert!(
            err.is_instance_of::<DimensionMismatchError>(py),
            "expected DimensionMismatchError, got {err:?}"
        );
    });
}

/// DimensionMismatch message includes expected and actual dimensions.
#[test]
fn test_core_err_dimension_mismatch_message() {
    pyo3::Python::initialize();
    Python::attach(|py| {
        let err = core_err(velesdb_core::Error::DimensionMismatch {
            expected: 768,
            actual: 512,
        });
        let msg = err.value(py).to_string();
        assert!(msg.contains("768"), "missing expected dim in: {msg}");
        assert!(msg.contains("512"), "missing actual dim in: {msg}");
    });
}

/// core_err_with_collection embeds the collection name in DimensionMismatch messages.
#[test]
fn test_core_err_with_collection_includes_name() {
    pyo3::Python::initialize();
    Python::attach(|py| {
        let err = core_err_with_collection(
            velesdb_core::Error::DimensionMismatch {
                expected: 768,
                actual: 512,
            },
            "docs",
        );
        assert!(
            err.is_instance_of::<DimensionMismatchError>(py),
            "expected DimensionMismatchError"
        );
        let msg = err.value(py).to_string();
        assert!(msg.contains("docs"), "missing collection name in: {msg}");
        assert!(msg.contains("768"), "missing expected dim in: {msg}");
        assert!(msg.contains("512"), "missing actual dim in: {msg}");
    });
}

/// CollectionNotFound maps to CollectionNotFoundError.
#[test]
fn test_core_err_collection_not_found_type() {
    pyo3::Python::initialize();
    Python::attach(|py| {
        let err = core_err(velesdb_core::Error::CollectionNotFound("my_col".into()));
        assert!(
            err.is_instance_of::<CollectionNotFoundError>(py),
            "expected CollectionNotFoundError, got {err:?}"
        );
        let msg = err.value(py).to_string();
        assert!(msg.contains("my_col"), "missing collection name in: {msg}");
    });
}

/// Engine/IO/internal errors fall back to VelesDBError so callers can
/// use `except velesdb.VelesDBError` as a catch-all.
#[test]
fn test_core_err_fallback_to_veles_db_error() {
    pyo3::Python::initialize();
    Python::attach(|py| {
        let err = core_err(velesdb_core::Error::Internal("oops".into()));
        assert!(
            err.is_instance_of::<VelesDBError>(py),
            "expected VelesDBError fallback, got {err:?}"
        );
        // Must NOT be mistaken for a typed subclass.
        assert!(
            !err.is_instance_of::<DimensionMismatchError>(py),
            "internal error should not be DimensionMismatchError"
        );
    });
}

// -------------------------------------------------------------------
// Commit 1 — hierarchy tests for the 3 newly introduced exceptions
//
// These tests only prove that the exception classes are correctly
// declared and that each subclass descends from `VelesDBError`. The
// mapping from `velesdb_core::Error` variants to these classes is
// wired in Commit 2 via `core_err`, and is tested there.
// -------------------------------------------------------------------

fn raise_instance<E>(py: Python<'_>, msg: &str) -> PyErr
where
    E: PyTypeInfo,
{
    let err = PyErr::new::<E, _>(msg.to_string());
    // Make sure the err is actually bound to the claimed type.
    assert!(
        err.is_instance_of::<E>(py),
        "raised {} but is not an instance of the expected type",
        std::any::type_name::<E>()
    );
    err
}

/// `CollectionExistsError` exists, carries its message, and inherits from `VelesDBError`.
#[test]
fn test_collection_exists_error_hierarchy() {
    pyo3::Python::initialize();
    Python::attach(|py| {
        let err = raise_instance::<CollectionExistsError>(py, "docs already exists");
        assert!(
            err.is_instance_of::<VelesDBError>(py),
            "CollectionExistsError must inherit from VelesDBError"
        );
        assert!(
            !err.is_instance_of::<pyo3::exceptions::PyRuntimeError>(py),
            "CollectionExistsError must NOT be a RuntimeError subclass"
        );
        assert!(err.value(py).to_string().contains("docs already exists"));
    });
}

/// `EdgeExistsError` exists, carries its message, and inherits from `VelesDBError`.
#[test]
fn test_edge_exists_error_hierarchy() {
    pyo3::Python::initialize();
    Python::attach(|py| {
        let err = raise_instance::<EdgeExistsError>(py, "edge 42 already present");
        assert!(
            err.is_instance_of::<VelesDBError>(py),
            "EdgeExistsError must inherit from VelesDBError"
        );
        assert!(err.value(py).to_string().contains("edge 42"));
    });
}

/// `DatabaseLockedError` exists, carries its message, and inherits from `VelesDBError`.
#[test]
fn test_database_locked_error_hierarchy() {
    pyo3::Python::initialize();
    Python::attach(|py| {
        let err = raise_instance::<DatabaseLockedError>(py, "held by pid 1234");
        assert!(
            err.is_instance_of::<VelesDBError>(py),
            "DatabaseLockedError must inherit from VelesDBError"
        );
        assert!(err.value(py).to_string().contains("pid 1234"));
    });
}

/// Regression guard: a generic `VelesDBError` must NOT be confused with
/// any of the more specific subclasses. This catches a refactor where
/// `create_exception!` macros accidentally collapse to the same type.
#[test]
fn test_veles_db_error_is_not_a_subclass() {
    pyo3::Python::initialize();
    Python::attach(|py| {
        let err = raise_instance::<VelesDBError>(py, "plain base error");
        assert!(!err.is_instance_of::<CollectionExistsError>(py));
        assert!(!err.is_instance_of::<EdgeExistsError>(py));
        assert!(!err.is_instance_of::<DatabaseLockedError>(py));
        assert!(!err.is_instance_of::<CollectionNotFoundError>(py));
        assert!(!err.is_instance_of::<DimensionMismatchError>(py));
    });
}

// -------------------------------------------------------------------
// Commit 2 — core_err mapping coverage for every VELES-XXX variant.
//
// Each test drives `core_err` with a concrete `velesdb_core::Error`
// variant and asserts it is converted to the semantically correct
// Python exception class. A single dispatch test at the bottom
// enumerates `Error::code()` across every known code so adding a
// new VELES-XXX code to core without updating `core_err` is caught
// by a compile-time exhaustiveness check on the helper constructor.
// -------------------------------------------------------------------

use pyo3::exceptions::{
    PyKeyError, PyMemoryError, PyOverflowError, PyPermissionError, PyValueError,
};
use pyo3::types::PyType;
use std::collections::BTreeSet;
use velesdb_core::Error as CoreError;

#[test]
fn test_core_err_collection_exists_type() {
    pyo3::Python::initialize();
    Python::attach(|py| {
        let err = core_err(CoreError::CollectionExists("docs".into()));
        assert!(err.is_instance_of::<CollectionExistsError>(py));
        assert!(err.is_instance_of::<VelesDBError>(py));
        assert!(err.value(py).to_string().contains("docs"));
    });
}

#[test]
fn test_core_err_edge_exists_type() {
    pyo3::Python::initialize();
    Python::attach(|py| {
        let err = core_err(CoreError::EdgeExists(42));
        assert!(err.is_instance_of::<EdgeExistsError>(py));
        assert!(err.is_instance_of::<VelesDBError>(py));
        assert!(err.value(py).to_string().contains("42"));
    });
}

#[test]
fn test_core_err_database_locked_type() {
    pyo3::Python::initialize();
    Python::attach(|py| {
        let err = core_err(CoreError::DatabaseLocked("pid 1234".into()));
        assert!(err.is_instance_of::<DatabaseLockedError>(py));
        assert!(err.is_instance_of::<VelesDBError>(py));
        assert!(err.value(py).to_string().contains("1234"));
    });
}

#[test]
fn test_core_err_point_not_found_is_key_error() {
    pyo3::Python::initialize();
    Python::attach(|py| {
        let err = core_err(CoreError::PointNotFound(7));
        assert!(err.is_instance_of::<PyKeyError>(py));
        assert!(err.value(py).to_string().contains('7'));
    });
}

#[test]
fn test_core_err_edge_not_found_is_key_error() {
    pyo3::Python::initialize();
    Python::attach(|py| {
        let err = core_err(CoreError::EdgeNotFound(99));
        assert!(err.is_instance_of::<PyKeyError>(py));
        assert!(err.value(py).to_string().contains("99"));
    });
}

#[test]
fn test_core_err_node_not_found_is_key_error() {
    pyo3::Python::initialize();
    Python::attach(|py| {
        let err = core_err(CoreError::NodeNotFound(11));
        assert!(err.is_instance_of::<PyKeyError>(py));
        assert!(err.value(py).to_string().contains("11"));
    });
}

#[test]
fn test_core_err_invalid_vector_is_value_error() {
    pyo3::Python::initialize();
    Python::attach(|py| {
        let err = core_err(CoreError::InvalidVector("nan at position 0".into()));
        assert!(err.is_instance_of::<PyValueError>(py));
    });
}

#[test]
fn test_core_err_query_is_value_error() {
    pyo3::Python::initialize();
    Python::attach(|py| {
        let err = core_err(CoreError::Query("unexpected token".into()));
        assert!(err.is_instance_of::<PyValueError>(py));
    });
}

#[test]
fn test_core_err_config_is_value_error() {
    pyo3::Python::initialize();
    Python::attach(|py| {
        let err = core_err(CoreError::Config("bad hnsw parameter".into()));
        assert!(err.is_instance_of::<PyValueError>(py));
    });
}

#[test]
fn test_core_err_graph_not_supported_is_value_error() {
    pyo3::Python::initialize();
    Python::attach(|py| {
        let err = core_err(CoreError::GraphNotSupported("edges disabled".into()));
        assert!(err.is_instance_of::<PyValueError>(py));
    });
}

#[test]
fn test_core_err_invalid_dimension_is_value_error() {
    pyo3::Python::initialize();
    Python::attach(|py| {
        let err = core_err(CoreError::InvalidDimension {
            dimension: 0,
            min: 1,
            max: 65536,
        });
        assert!(err.is_instance_of::<PyValueError>(py));
    });
}

#[test]
fn test_core_err_invalid_collection_name_is_value_error() {
    pyo3::Python::initialize();
    Python::attach(|py| {
        let err = core_err(CoreError::InvalidCollectionName {
            name: "../etc/passwd".into(),
            reason: "path traversal".into(),
        });
        assert!(err.is_instance_of::<PyValueError>(py));
    });
}

#[test]
fn test_core_err_overflow_is_overflow_error() {
    pyo3::Python::initialize();
    Python::attach(|py| {
        let err = core_err(CoreError::Overflow("u64 -> usize".into()));
        assert!(err.is_instance_of::<PyOverflowError>(py));
    });
}

#[test]
fn test_core_err_allocation_failed_is_memory_error() {
    pyo3::Python::initialize();
    Python::attach(|py| {
        let err = core_err(CoreError::AllocationFailed("16 GiB".into()));
        assert!(err.is_instance_of::<PyMemoryError>(py));
    });
}

/// Storage errors map to VelesDBError so `except velesdb.VelesDBError` catches them.
#[test]
fn test_core_err_storage_is_veles_db_error() {
    pyo3::Python::initialize();
    Python::attach(|py| {
        let err = core_err(CoreError::Storage("disk full".into()));
        assert!(
            err.is_instance_of::<VelesDBError>(py),
            "Storage must map to VelesDBError so except velesdb.VelesDBError catches it"
        );
        // Must not be confused with any specific subclass.
        assert!(!err.is_instance_of::<DimensionMismatchError>(py));
        assert!(!err.is_instance_of::<CollectionNotFoundError>(py));
    });
}

/// Internal errors map to VelesDBError (same as Storage, Io, etc.).
#[test]
fn test_core_err_internal_is_veles_db_error() {
    pyo3::Python::initialize();
    Python::attach(|py| {
        let err = core_err(CoreError::Internal("BUG: unreachable".into()));
        assert!(err.is_instance_of::<VelesDBError>(py));
    });
}

/// One error of every `velesdb_core::Error` variant, each beside the Python
/// class it must raise as — the mapping table on `core_err`.
///
/// `Error` is `#[non_exhaustive]`, so no match outside core can be
/// exhaustive, and a variant core adds compiles here unlisted. The guard is
/// [`test_every_documented_code_is_listed`] instead: the codes listed here
/// must equal the codes `docs/reference/ERROR_CODES.md` documents, which
/// `scripts/check-error-codes.py` keeps equal to the codes core declares —
/// so a new variant fails here until it is listed, with its class.
fn every_variant_with_class(py: Python<'_>) -> Vec<(CoreError, Bound<'_, PyType>)> {
    let value = PyValueError::type_object(py);
    let key = PyKeyError::type_object(py);
    let veles = VelesDBError::type_object(py);
    vec![
        (
            CoreError::CollectionExists("x".into()),
            CollectionExistsError::type_object(py),
        ),
        (
            CoreError::CollectionNotFound("x".into()),
            CollectionNotFoundError::type_object(py),
        ),
        (CoreError::PointNotFound(1), key.clone()),
        (
            CoreError::DimensionMismatch {
                expected: 1,
                actual: 2,
            },
            DimensionMismatchError::type_object(py),
        ),
        (CoreError::InvalidVector("x".into()), value.clone()),
        (CoreError::Storage("x".into()), veles.clone()),
        (CoreError::Index("x".into()), veles.clone()),
        (CoreError::IndexCorrupted("x".into()), veles.clone()),
        (CoreError::Config("x".into()), value.clone()),
        (CoreError::Query("x".into()), value.clone()),
        (
            CoreError::Io(std::io::Error::other("disk error")),
            veles.clone(),
        ),
        (CoreError::Serialization("x".into()), veles.clone()),
        (CoreError::Internal("x".into()), veles.clone()),
        (CoreError::VectorNotAllowed("x".into()), value.clone()),
        (CoreError::SearchNotSupported("x".into()), value.clone()),
        (CoreError::VectorRequired("x".into()), value.clone()),
        (CoreError::SchemaValidation("x".into()), value.clone()),
        (CoreError::GraphNotSupported("x".into()), value.clone()),
        (CoreError::EdgeExists(1), EdgeExistsError::type_object(py)),
        (CoreError::EdgeNotFound(1), key.clone()),
        (CoreError::InvalidEdgeLabel("x".into()), value.clone()),
        (CoreError::NodeNotFound(1), key),
        (
            CoreError::Overflow("x".into()),
            PyOverflowError::type_object(py),
        ),
        (CoreError::ColumnStoreError("x".into()), veles.clone()),
        (CoreError::GpuError("x".into()), veles.clone()),
        (CoreError::EpochMismatch("x".into()), veles.clone()),
        (
            CoreError::GuardRail {
                kind: velesdb_core::GuardRailKind::Cardinality,
                message: "x".into(),
            },
            veles.clone(),
        ),
        (CoreError::InvalidQuantizerConfig("x".into()), value.clone()),
        (CoreError::TrainingFailed("x".into()), veles.clone()),
        (CoreError::SparseIndexError("x".into()), veles.clone()),
        (
            CoreError::DatabaseLocked("x".into()),
            DatabaseLockedError::type_object(py),
        ),
        (
            CoreError::InvalidDimension {
                dimension: 0,
                min: 1,
                max: 2,
            },
            value.clone(),
        ),
        (
            CoreError::AllocationFailed("x".into()),
            PyMemoryError::type_object(py),
        ),
        (
            CoreError::InvalidCollectionName {
                name: "x".into(),
                reason: "y".into(),
            },
            value,
        ),
        (CoreError::SnapshotBuildFailed("x".into()), veles.clone()),
        (
            CoreError::IncompatibleSchemaVersion {
                found: 2,
                supported: 1,
            },
            veles,
        ),
        (
            CoreError::AccessDenied("x".into()),
            PyPermissionError::type_object(py),
        ),
        (
            CoreError::QueryParameter {
                name: "v".into(),
                reason: "not bound".into(),
            },
            crate::velesql::VelesQLParameterError::type_object(py),
        ),
    ]
}

/// Every variant raises exactly the class the mapping table gives it — not
/// a subclass, not the `VelesDBError` fallback in its place.
#[test]
fn test_core_err_raises_the_class_of_every_variant() {
    pyo3::Python::initialize();
    Python::attach(|py| {
        for (case, class) in every_variant_with_class(py) {
            let code = case.code();
            let raised = core_err(case).get_type(py);
            assert!(
                raised.is(&class),
                "core_err({code}) raised {raised:?}, the table says {class:?}"
            );
        }
    });
}

/// The variants listed above are every code core documents: a code added to
/// `ERROR_CODES.md` (and so to core, `scripts/check-error-codes.py`) fails
/// here until its variant is listed with its class.
#[test]
fn test_every_documented_code_is_listed() {
    pyo3::Python::initialize();
    Python::attach(|py| {
        let listed: BTreeSet<&str> = every_variant_with_class(py)
            .iter()
            .map(|(case, _)| case.code())
            .collect();
        assert_eq!(listed, documented_codes());
    });
}

/// Every `VELES-XXX` code `docs/reference/ERROR_CODES.md` names.
fn documented_codes() -> BTreeSet<&'static str> {
    const DOC: &str = include_str!("../../../docs/reference/ERROR_CODES.md");
    DOC.match_indices("VELES-")
        .filter_map(|(at, _)| DOC.get(at..at + 9))
        .filter(|code| code[6..].bytes().all(|b| b.is_ascii_digit()))
        .collect()
}

/// Every code `PYTHON_CLASSES` maps is one a variant returns: a typo there
/// would leave its variant on the `VelesDBError` fallback with no other red.
#[test]
fn test_every_mapped_code_belongs_to_a_variant() {
    pyo3::Python::initialize();
    Python::attach(|py| {
        let codes: BTreeSet<&str> = every_variant_with_class(py)
            .iter()
            .map(|(case, _)| case.code())
            .collect();
        for &(code, _) in crate::collection_helpers::PYTHON_CLASSES {
            assert!(
                codes.contains(code),
                "PYTHON_CLASSES maps {code}, which no variant returns"
            );
        }
    });
}

/// A parameter error (VELES-038) raises `VelesQLParameterError`, which is a
/// `ValueError`; any other query error (VELES-010) stays a plain
/// `ValueError`, never the parameter class.
#[test]
fn test_core_err_query_parameter_is_velesql_parameter_error() {
    use crate::velesql::VelesQLParameterError;
    pyo3::Python::initialize();
    Python::attach(|py| {
        let err = core_err(CoreError::QueryParameter {
            name: "v".into(),
            reason: "not bound".into(),
        });
        assert!(err.is_instance_of::<VelesQLParameterError>(py), "{err:?}");
        assert!(err.is_instance_of::<PyValueError>(py), "{err:?}");
        assert!(err.to_string().contains("[VELES-038]"), "{err}");

        let query = core_err(CoreError::Query("[V015] bad ORDER BY".into()));
        assert!(query.is_instance_of::<PyValueError>(py), "{query:?}");
        assert!(
            !query.is_instance_of::<VelesQLParameterError>(py),
            "{query:?}"
        );
    });
}
