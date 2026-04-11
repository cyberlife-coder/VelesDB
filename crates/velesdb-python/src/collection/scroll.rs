//! Scroll cursor for paginated iteration over collection points.
//!
//! Provides `Collection.scroll()` — a Python generator that yields batches
//! of points via the Rust-native `scroll_batch` method.

use pyo3::exceptions::{PyImportError, PyValueError};
use pyo3::prelude::*;

use crate::collection::Collection;
use crate::collection_helpers::{core_err, parse_optional_filter, point_to_dict};

/// Python iterator that yields batches from `scroll_batch`.
#[pyclass]
pub(crate) struct ScrollIterator {
    /// Clone of the inner collection (cheap — Arc-wrapped).
    inner: velesdb_core::VectorCollection,
    /// Current cursor position (`None` = start).
    cursor: Option<u64>,
    /// Maximum points per batch.
    batch_size: usize,
    /// Optional payload filter.
    filter: Option<velesdb_core::Filter>,
    /// Whether to convert batches to DataFrames.
    as_dataframe: bool,
    /// Backend name ("pandas" or "polars").
    backend: String,
    /// Whether iteration is complete.
    exhausted: bool,
}

#[pymethods]
impl ScrollIterator {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    /// Yield the next batch of points.
    ///
    /// Releases the GIL during the actual disk/mmap read performed by
    /// `scroll_batch`. This is item #17 of Sprint 2 Wave 3 — before
    /// Commit 3 the GIL was held for the entire batch read, so two
    /// Python threads scrolling two different collections serialised
    /// through the interpreter instead of progressing in parallel.
    ///
    /// The filter is cloned into an owned `Option<Filter>` before
    /// crossing the `allow_threads` boundary because `py.allow_threads`
    /// requires a `'static + Send` closure: a borrow of `self.filter`
    /// would otherwise be tied to the `&mut self` outer lifetime. The
    /// `Filter` clone is cheap (the structure is shallow and shared
    /// where it can be) and pays for itself many times over by
    /// releasing the GIL for the duration of the page read, which is
    /// the dominant cost of a scroll step.
    fn __next__(&mut self, py: Python<'_>) -> PyResult<Option<PyObject>> {
        if self.exhausted {
            return Ok(None);
        }

        // Snapshot inputs for the blocking closure. `inner` is already
        // `Arc`-backed so the clone is a ref-count bump.
        let inner = self.inner.clone();
        let cursor = self.cursor;
        let batch_size = self.batch_size;
        let filter_owned = self.filter.clone();

        // Release the GIL while the core walks the mmap region and
        // applies the optional filter. Any resulting `velesdb_core::Error`
        // is routed to the typed Python exception hierarchy by `core_err`.
        let batch = py
            .allow_threads(move || inner.scroll_batch(cursor, batch_size, filter_owned.as_ref()))
            .map_err(core_err)?;

        if batch.points.is_empty() {
            self.exhausted = true;
            return Ok(None);
        }

        // next_cursor is always Some(id) for non-empty batches (points.last().map(|p| p.id)).
        // Exhaustion is handled via the empty-batch early return above.
        self.cursor = batch.next_cursor;

        let dicts: Vec<PyObject> = batch.points.iter().map(|p| point_to_dict(py, p)).collect();
        let py_list = pyo3::types::PyList::new(py, &dicts)?;

        if self.as_dataframe {
            let converter = py.import("velesdb.dataframe_converter")?;
            let df = converter.call_method1("to_scroll_dataframe", (py_list, &self.backend))?;
            Ok(Some(df.unbind()))
        } else {
            Ok(Some(py_list.into_any().unbind()))
        }
    }
}

#[pymethods]
impl Collection {
    /// Yield batches of points from the collection.
    ///
    /// Args:
    ///     batch_size: Points per batch (default 100).
    ///     filter: Optional payload filter dict.
    ///     as_dataframe: If True, yield DataFrames instead of list\[dict\].
    ///     backend: "pandas" or "polars" (default "pandas").
    ///
    /// Returns:
    ///     Iterator of batches (list\[dict\] or DataFrame).
    ///
    /// Raises:
    ///     ValueError: If batch_size is 0.
    ///     ImportError: If as_dataframe=True and backend is not installed.
    #[pyo3(signature = (*, batch_size=100, filter=None, as_dataframe=false, backend="pandas"))]
    fn scroll(
        &self,
        py: Python<'_>,
        batch_size: usize,
        filter: Option<PyObject>,
        as_dataframe: bool,
        backend: &str,
    ) -> PyResult<ScrollIterator> {
        if batch_size == 0 {
            return Err(PyValueError::new_err("batch_size must be greater than 0"));
        }

        if as_dataframe {
            validate_backend_installed(py, backend)?;
        }

        let parsed_filter = parse_optional_filter(py, filter)?;

        Ok(ScrollIterator {
            inner: self.inner.clone(),
            cursor: None,
            batch_size,
            filter: parsed_filter,
            as_dataframe,
            backend: backend.to_string(),
            exhausted: false,
        })
    }
}

/// Eagerly validate that the requested DataFrame backend is importable.
fn validate_backend_installed(py: Python<'_>, backend: &str) -> PyResult<()> {
    match backend {
        "pandas" => {
            py.import("pandas").map_err(|_| {
                PyImportError::new_err(
                    "pandas is required for DataFrame support. Install it with: pip install velesdb[pandas]",
                )
            })?;
        }
        "polars" => {
            py.import("polars").map_err(|_| {
                PyImportError::new_err(
                    "polars is required for DataFrame support. Install it with: pip install velesdb[polars]",
                )
            })?;
        }
        _ => {
            return Err(PyValueError::new_err(format!(
                "Unsupported backend '{backend}'. Use 'pandas' or 'polars'"
            )));
        }
    }
    Ok(())
}
