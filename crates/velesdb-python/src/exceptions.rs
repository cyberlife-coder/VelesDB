//! Python exception hierarchy for VelesDB.
//!
//! Provides typed exceptions that map 1-to-1 with the most actionable
//! `velesdb_core::Error` variants, so Python callers can `except` specific
//! error classes instead of catching a generic `RuntimeError`.
//!
//! # Hierarchy
//!
//! ```text
//! Exception
//! └── VelesDBError              — base for all core operation errors
//!     ├── DimensionMismatchError    (VELES-004)
//!     ├── CollectionNotFoundError   (VELES-002)
//!     ├── CollectionExistsError     (VELES-001)
//!     ├── EdgeExistsError           (VELES-019)
//!     └── DatabaseLockedError       (VELES-031)
//! ```
//!
//! Every subclass inherits from [`VelesDBError`], so Python callers that
//! want a catch-all can write `except velesdb.VelesDBError`. Specific
//! handlers that need to discriminate (for example to retry a locked
//! database or surface a collection conflict as a user-facing error)
//! should catch the specific subclass instead.
//!
//! # Example (Python)
//!
//! ```python
//! import velesdb
//!
//! try:
//!     collection.upsert([{"id": 1, "vector": short_vec}])
//! except velesdb.DimensionMismatchError as e:
//!     print(e)  # Expected 768 dimensions, got 512 (collection 'docs' requires 768-dim vectors)
//! except velesdb.CollectionExistsError:
//!     print("collection already created — skipping")
//! except velesdb.DatabaseLockedError:
//!     print("another process holds the database lock")
//! except velesdb.VelesDBError as e:
//!     print(f"VelesDB error: {e}")
//! ```

use pyo3::prelude::*;

// Base exception for all VelesDB core operation errors.
pyo3::create_exception!(velesdb, VelesDBError, pyo3::exceptions::PyException);

// Raised when the vector dimension does not match the collection's configured dimension.
pyo3::create_exception!(velesdb, DimensionMismatchError, VelesDBError);

// Raised when the referenced collection does not exist.
pyo3::create_exception!(velesdb, CollectionNotFoundError, VelesDBError);

// Raised when a collection with the requested name already exists.
pyo3::create_exception!(velesdb, CollectionExistsError, VelesDBError);

// Raised when an edge with the requested ID already exists in a graph collection.
pyo3::create_exception!(velesdb, EdgeExistsError, VelesDBError);

// Raised when the database directory is held by another process (file lock).
pyo3::create_exception!(velesdb, DatabaseLockedError, VelesDBError);

/// Register all VelesDB exception types with the Python module.
///
/// Must be called from `lib.rs::velesdb()` module initializer. Every
/// exception registered here must also appear in the Python facade
/// `python/velesdb/__init__.py` import list and `__all__` export tuple,
/// otherwise Python callers cannot reach the typed classes via
/// `import velesdb` (they remain accessible via the raw extension
/// module `velesdb.velesdb`, but that path is not part of the public
/// API surface).
pub fn register_exceptions(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("VelesDBError", m.py().get_type::<VelesDBError>())?;
    m.add(
        "DimensionMismatchError",
        m.py().get_type::<DimensionMismatchError>(),
    )?;
    m.add(
        "CollectionNotFoundError",
        m.py().get_type::<CollectionNotFoundError>(),
    )?;
    m.add(
        "CollectionExistsError",
        m.py().get_type::<CollectionExistsError>(),
    )?;
    m.add("EdgeExistsError", m.py().get_type::<EdgeExistsError>())?;
    m.add(
        "DatabaseLockedError",
        m.py().get_type::<DatabaseLockedError>(),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collection_helpers::{core_err, core_err_with_collection};
    use pyo3::{PyTypeInfo, Python};

    /// DimensionMismatch maps to DimensionMismatchError, not RuntimeError.
    #[test]
    fn test_core_err_dimension_mismatch_type() {
        pyo3::prepare_freethreaded_python();
        Python::with_gil(|py| {
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
        pyo3::prepare_freethreaded_python();
        Python::with_gil(|py| {
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
        pyo3::prepare_freethreaded_python();
        Python::with_gil(|py| {
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
        pyo3::prepare_freethreaded_python();
        Python::with_gil(|py| {
            let err = core_err(velesdb_core::Error::CollectionNotFound("my_col".into()));
            assert!(
                err.is_instance_of::<CollectionNotFoundError>(py),
                "expected CollectionNotFoundError, got {err:?}"
            );
            let msg = err.value(py).to_string();
            assert!(msg.contains("my_col"), "missing collection name in: {msg}");
        });
    }

    /// Other errors fall back to PyRuntimeError, not a VelesDBError subclass.
    #[test]
    fn test_core_err_fallback_to_runtime_error() {
        pyo3::prepare_freethreaded_python();
        Python::with_gil(|py| {
            let err = core_err(velesdb_core::Error::Internal("oops".into()));
            assert!(
                err.is_instance_of::<pyo3::exceptions::PyRuntimeError>(py),
                "expected PyRuntimeError fallback, got {err:?}"
            );
            // Must NOT be mistaken for a typed VelesDB error.
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
        pyo3::prepare_freethreaded_python();
        Python::with_gil(|py| {
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
        pyo3::prepare_freethreaded_python();
        Python::with_gil(|py| {
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
        pyo3::prepare_freethreaded_python();
        Python::with_gil(|py| {
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
        pyo3::prepare_freethreaded_python();
        Python::with_gil(|py| {
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
        PyKeyError, PyMemoryError, PyOverflowError, PyRuntimeError, PyValueError,
    };
    use velesdb_core::Error as CoreError;

    #[test]
    fn test_core_err_collection_exists_type() {
        pyo3::prepare_freethreaded_python();
        Python::with_gil(|py| {
            let err = core_err(CoreError::CollectionExists("docs".into()));
            assert!(err.is_instance_of::<CollectionExistsError>(py));
            assert!(err.is_instance_of::<VelesDBError>(py));
            assert!(err.value(py).to_string().contains("docs"));
        });
    }

    #[test]
    fn test_core_err_edge_exists_type() {
        pyo3::prepare_freethreaded_python();
        Python::with_gil(|py| {
            let err = core_err(CoreError::EdgeExists(42));
            assert!(err.is_instance_of::<EdgeExistsError>(py));
            assert!(err.is_instance_of::<VelesDBError>(py));
            assert!(err.value(py).to_string().contains("42"));
        });
    }

    #[test]
    fn test_core_err_database_locked_type() {
        pyo3::prepare_freethreaded_python();
        Python::with_gil(|py| {
            let err = core_err(CoreError::DatabaseLocked("pid 1234".into()));
            assert!(err.is_instance_of::<DatabaseLockedError>(py));
            assert!(err.is_instance_of::<VelesDBError>(py));
            assert!(err.value(py).to_string().contains("1234"));
        });
    }

    #[test]
    fn test_core_err_point_not_found_is_key_error() {
        pyo3::prepare_freethreaded_python();
        Python::with_gil(|py| {
            let err = core_err(CoreError::PointNotFound(7));
            assert!(err.is_instance_of::<PyKeyError>(py));
            assert!(err.value(py).to_string().contains('7'));
        });
    }

    #[test]
    fn test_core_err_edge_not_found_is_key_error() {
        pyo3::prepare_freethreaded_python();
        Python::with_gil(|py| {
            let err = core_err(CoreError::EdgeNotFound(99));
            assert!(err.is_instance_of::<PyKeyError>(py));
            assert!(err.value(py).to_string().contains("99"));
        });
    }

    #[test]
    fn test_core_err_node_not_found_is_key_error() {
        pyo3::prepare_freethreaded_python();
        Python::with_gil(|py| {
            let err = core_err(CoreError::NodeNotFound(11));
            assert!(err.is_instance_of::<PyKeyError>(py));
            assert!(err.value(py).to_string().contains("11"));
        });
    }

    #[test]
    fn test_core_err_invalid_vector_is_value_error() {
        pyo3::prepare_freethreaded_python();
        Python::with_gil(|py| {
            let err = core_err(CoreError::InvalidVector("nan at position 0".into()));
            assert!(err.is_instance_of::<PyValueError>(py));
        });
    }

    #[test]
    fn test_core_err_query_is_value_error() {
        pyo3::prepare_freethreaded_python();
        Python::with_gil(|py| {
            let err = core_err(CoreError::Query("unexpected token".into()));
            assert!(err.is_instance_of::<PyValueError>(py));
        });
    }

    #[test]
    fn test_core_err_config_is_value_error() {
        pyo3::prepare_freethreaded_python();
        Python::with_gil(|py| {
            let err = core_err(CoreError::Config("bad hnsw parameter".into()));
            assert!(err.is_instance_of::<PyValueError>(py));
        });
    }

    #[test]
    fn test_core_err_graph_not_supported_is_value_error() {
        pyo3::prepare_freethreaded_python();
        Python::with_gil(|py| {
            let err = core_err(CoreError::GraphNotSupported("edges disabled".into()));
            assert!(err.is_instance_of::<PyValueError>(py));
        });
    }

    #[test]
    fn test_core_err_invalid_dimension_is_value_error() {
        pyo3::prepare_freethreaded_python();
        Python::with_gil(|py| {
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
        pyo3::prepare_freethreaded_python();
        Python::with_gil(|py| {
            let err = core_err(CoreError::InvalidCollectionName {
                name: "../etc/passwd".into(),
                reason: "path traversal".into(),
            });
            assert!(err.is_instance_of::<PyValueError>(py));
        });
    }

    #[test]
    fn test_core_err_overflow_is_overflow_error() {
        pyo3::prepare_freethreaded_python();
        Python::with_gil(|py| {
            let err = core_err(CoreError::Overflow("u64 -> usize".into()));
            assert!(err.is_instance_of::<PyOverflowError>(py));
        });
    }

    #[test]
    fn test_core_err_allocation_failed_is_memory_error() {
        pyo3::prepare_freethreaded_python();
        Python::with_gil(|py| {
            let err = core_err(CoreError::AllocationFailed("16 GiB".into()));
            assert!(err.is_instance_of::<PyMemoryError>(py));
        });
    }

    #[test]
    fn test_core_err_storage_is_runtime_error() {
        pyo3::prepare_freethreaded_python();
        Python::with_gil(|py| {
            let err = core_err(CoreError::Storage("disk full".into()));
            assert!(err.is_instance_of::<PyRuntimeError>(py));
            // Engine errors stay as generic RuntimeError and must NOT be
            // confused with any of the typed VelesDB subclasses.
            assert!(!err.is_instance_of::<VelesDBError>(py));
        });
    }

    #[test]
    fn test_core_err_internal_is_runtime_error() {
        pyo3::prepare_freethreaded_python();
        Python::with_gil(|py| {
            let err = core_err(CoreError::Internal("BUG: unreachable".into()));
            assert!(err.is_instance_of::<PyRuntimeError>(py));
        });
    }

    /// Exhaustiveness guard: every `VELES-XXX` code currently defined in
    /// `velesdb_core::Error::code()` must map to a known Python exception.
    ///
    /// This walks a representative instance of every variant and asserts
    /// that `core_err` produces one of the expected concrete types. If a
    /// new variant is added to core, the match in this test becomes
    /// non-exhaustive at compile time, forcing the author to update both
    /// the mapping in `collection_helpers::core_err` and this test.
    #[test]
    fn test_core_err_mapping_covers_every_code() {
        pyo3::prepare_freethreaded_python();
        Python::with_gil(|py| {
            let cases: Vec<CoreError> = vec![
                CoreError::CollectionExists("x".into()),
                CoreError::CollectionNotFound("x".into()),
                CoreError::PointNotFound(1),
                CoreError::DimensionMismatch {
                    expected: 1,
                    actual: 2,
                },
                CoreError::InvalidVector("x".into()),
                CoreError::Storage("x".into()),
                CoreError::Index("x".into()),
                CoreError::IndexCorrupted("x".into()),
                CoreError::Config("x".into()),
                CoreError::Query("x".into()),
                CoreError::Serialization("x".into()),
                CoreError::Internal("x".into()),
                CoreError::VectorNotAllowed("x".into()),
                CoreError::SearchNotSupported("x".into()),
                CoreError::VectorRequired("x".into()),
                CoreError::SchemaValidation("x".into()),
                CoreError::GraphNotSupported("x".into()),
                CoreError::EdgeExists(1),
                CoreError::EdgeNotFound(1),
                CoreError::InvalidEdgeLabel("x".into()),
                CoreError::NodeNotFound(1),
                CoreError::Overflow("x".into()),
                CoreError::ColumnStoreError("x".into()),
                CoreError::GpuError("x".into()),
                CoreError::EpochMismatch("x".into()),
                CoreError::GuardRail("x".into()),
                CoreError::InvalidQuantizerConfig("x".into()),
                CoreError::TrainingFailed("x".into()),
                CoreError::SparseIndexError("x".into()),
                CoreError::DatabaseLocked("x".into()),
                CoreError::InvalidDimension {
                    dimension: 0,
                    min: 1,
                    max: 2,
                },
                CoreError::AllocationFailed("x".into()),
                CoreError::InvalidCollectionName {
                    name: "x".into(),
                    reason: "y".into(),
                },
                CoreError::SnapshotBuildFailed("x".into()),
                CoreError::IncompatibleSchemaVersion {
                    found: 2,
                    supported: 1,
                },
            ];
            // VELES-011 (Io) requires a std::io::Error which we construct
            // explicitly rather than inline into the vec literal.
            let io_case = CoreError::Io(std::io::Error::other("disk error"));
            let mut all_cases = cases;
            all_cases.push(io_case);

            for case in all_cases {
                let code = case.code();
                let err = core_err(case);
                // Every variant must map to EITHER a typed VelesDBError
                // subclass OR one of the canonical Python builtins. Nothing
                // is allowed to produce a pure Python `Exception` without a
                // more specific type.
                let typed = err.is_instance_of::<VelesDBError>(py)
                    || err.is_instance_of::<PyKeyError>(py)
                    || err.is_instance_of::<PyValueError>(py)
                    || err.is_instance_of::<PyOverflowError>(py)
                    || err.is_instance_of::<PyMemoryError>(py)
                    || err.is_instance_of::<PyRuntimeError>(py);
                assert!(
                    typed,
                    "core_err({code}) produced an untyped exception: {err:?}"
                );
            }
        });
    }
}
