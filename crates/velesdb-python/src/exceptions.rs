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
#[path = "exceptions_tests.rs"]
mod tests;
