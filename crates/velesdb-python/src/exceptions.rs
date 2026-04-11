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
}
