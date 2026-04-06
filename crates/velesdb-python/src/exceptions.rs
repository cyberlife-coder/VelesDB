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
//! └── VelesDBError          — base for all core operation errors
//!     ├── DimensionMismatchError   (VELES-004)
//!     └── CollectionNotFoundError  (VELES-002)
//! ```
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

/// Register all VelesDB exception types with the Python module.
///
/// Must be called from `lib.rs::velesdb()` module initializer.
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
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collection_helpers::{core_err, core_err_with_collection};
    use pyo3::Python;

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
}
