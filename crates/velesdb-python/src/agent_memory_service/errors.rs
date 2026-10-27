//! How a `MemoryError` becomes the Python exception its category calls for.

use pyo3::exceptions::{PyKeyError, PyNotImplementedError, PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use velesdb_memory::{ErrorCategory, MemoryError};

/// Map a [`MemoryError`] to the most specific Python exception, driven by its
/// transport-neutral [`ErrorCategory`] so the taxonomy stays identical to the
/// MCP server and the Node binding: caller-input → `ValueError`, a missing
/// memory id → `KeyError`, the rest → `RuntimeError`.
pub(super) fn to_py_err(e: MemoryError) -> PyErr {
    let msg = e.to_string();
    known_category_err(e.category(), msg.clone()).unwrap_or_else(|| PyRuntimeError::new_err(msg))
}

/// The explicit half of the mapping, split from the runtime fallback
/// (`RuntimeError`, the coarsest bucket) so the two are distinguishable.
///
/// `ErrorCategory` is `non_exhaustive`, so a new category no longer fails this
/// match at compile time. Unlike the Node and wasm adapters, no local test
/// walks [`ErrorCategory::ALL`] here: this crate's Rust tests never run (PyO3
/// linkage keeps it out of `cargo test --workspace`, see `AGENTS.md`). The
/// guard is transitive instead — a new category turns the Node and wasm
/// coverage tests red in the same workspace CI that gates this wheel, and
/// their cross-reference leads whoever fixes them here. Keep the three in sync.
fn known_category_err(category: ErrorCategory, msg: String) -> Option<PyErr> {
    Some(match category {
        ErrorCategory::InvalidInput => PyValueError::new_err(msg),
        ErrorCategory::NotFound => PyKeyError::new_err(msg),
        ErrorCategory::Internal => PyRuntimeError::new_err(msg),
        ErrorCategory::Unsupported => PyNotImplementedError::new_err(msg),
        _ => return None,
    })
}

/// A startup setting the caller got wrong: always a `ValueError` to correct.
pub(super) fn setting_error(error: velesdb_memory::SettingError) -> PyErr {
    PyValueError::new_err(error.to_string())
}
