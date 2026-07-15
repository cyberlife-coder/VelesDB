//! Scroll cursor for paginated iteration over collection points.
//!
//! Provides `Collection.scroll()` — a Python generator that yields batches
//! of points via the Rust-native `scroll_batch` method.

use std::sync::Arc;

use pyo3::exceptions::{PyImportError, PyValueError};
use pyo3::prelude::*;
use velesdb_core::{Database as CoreDatabase, QueryOperationKind};

use crate::collection::Collection;
use crate::collection_helpers::{core_err, parse_optional_filter, point_to_dict};

use super::and_scope;

/// Python iterator that yields batches from `scroll_batch`.
#[pyclass]
pub(crate) struct ScrollIterator {
    /// Clone of the inner collection (cheap — Arc-wrapped).
    inner: velesdb_core::VectorCollection,
    /// Shared handle to the owning database. `inner` is a detached leaf with
    /// no observer reference, so every batch read consults this handle's
    /// control-plane read gate (`authorize_read`) — a long-lived cursor keeps
    /// honoring governance decisions made after it was created (#1405).
    db: Arc<CoreDatabase>,
    /// Collection name the gate is keyed on.
    name: String,
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
    fn __next__(&mut self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        if self.exhausted {
            return Ok(None);
        }

        // Snapshot inputs for the blocking closure. `inner` is already
        // `Arc`-backed so the clone is a ref-count bump.
        let inner = self.inner.clone();
        let db = Arc::clone(&self.db);
        let name = self.name.clone();
        let cursor = self.cursor;
        let batch_size = self.batch_size;
        let filter_owned = self.filter.clone();

        // Release the GIL while the core walks the mmap region and
        // applies the optional filter. Any resulting `velesdb_core::Error`
        // is routed to the typed Python exception hierarchy by `core_err`.
        //
        // Each batch consults the read gate: deny fails closed mid-iteration,
        // and an observer scope filter is AND-composed with the caller filter
        // so the batch read only narrows. With no observer this is a single
        // `Option` check (zero-overhead allow, behavior unchanged).
        let batch = py.detach(move || {
            let scope = db
                .authorize_read(&name, QueryOperationKind::Select, None, None)
                .map_err(core_err)?;
            let effective = and_scope(filter_owned, scope);
            inner
                .scroll_batch(cursor, batch_size, effective.as_ref())
                .map_err(core_err)
        })?;

        if batch.points.is_empty() {
            self.exhausted = true;
            return Ok(None);
        }

        // next_cursor is always Some(id) for non-empty batches (points.last().map(|p| p.id)).
        // Exhaustion is handled via the empty-batch early return above.
        self.cursor = batch.next_cursor;

        let dicts: Vec<Py<PyAny>> = batch.points.iter().map(|p| point_to_dict(py, p)).collect();
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
        filter: Option<Py<PyAny>>,
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
            db: Arc::clone(&self.db),
            name: self.name.clone(),
            cursor: None,
            batch_size,
            filter: parsed_filter,
            as_dataframe,
            backend: backend.to_string(),
            exhausted: false,
        })
    }

    /// Fetch one batch of points starting after `cursor` (O(1) cursor seek).
    ///
    /// Returns ``(points, next_cursor)``: the batch as a list of dicts and the
    /// cursor to pass on the next call (``None`` once the batch is the last).
    /// Unlike the [`scroll`](Self::scroll) generator this is a stateless
    /// one-shot for callers that persist the cursor themselves (e.g.
    /// ``velesdb_common.scroll``), avoiding an O(n) re-scan per page.
    #[pyo3(signature = (cursor=None, batch_size=100, filter=None))]
    fn scroll_batch(
        &self,
        py: Python<'_>,
        cursor: Option<u64>,
        batch_size: usize,
        filter: Option<Py<PyAny>>,
    ) -> PyResult<(Vec<Py<PyAny>>, Option<u64>)> {
        if batch_size == 0 {
            return Err(PyValueError::new_err("batch_size must be greater than 0"));
        }

        let parsed_filter = parse_optional_filter(py, filter)?;
        let inner = self.inner.clone();
        // Stateless page reads pass the same read gate as the scroll iterator:
        // deny fails closed, an observer scope filter AND-composes with the
        // caller filter (narrow-only). No observer ⇒ unchanged single check.
        let batch = py.detach(move || {
            let scope = self.authorize(QueryOperationKind::Select, None, None)?;
            let effective = and_scope(parsed_filter, scope);
            inner
                .scroll_batch(cursor, batch_size, effective.as_ref())
                .map_err(core_err)
        })?;

        let dicts: Vec<Py<PyAny>> = batch.points.iter().map(|p| point_to_dict(py, p)).collect();
        Ok((dicts, batch.next_cursor))
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
