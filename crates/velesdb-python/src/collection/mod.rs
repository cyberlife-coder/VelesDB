//! Collection module for VelesDB Python bindings.
//!
//! Split into focused sub-modules:
//! - `search` — search methods (dense, sparse, hybrid, batch, multi-query)
//! - `query` — VelesQL query/match/explain methods
//! - `mutation` — upsert, delete, flush, stream_insert
//! - `index` — index CRUD (property, range)
//!
//! Note: Multiple `#[pymethods]` impl blocks across sub-modules are intentional.
//! PyO3 >= 0.21 supports this natively via inventory-based method registration.
//! rust-analyzer may incorrectly flag `PyMethods` trait conflicts — verify with `cargo build`.

mod dataframe;
mod index;
mod mutation;
pub(crate) mod query;
pub(crate) mod scroll;
mod search;

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyString};
use velesdb_core::{
    Filter, FusionStrategy as CoreFusionStrategy, SearchResult, VectorCollection as CoreCollection,
};

/// Default fusion strategy when none is specified by the caller.
const DEFAULT_FUSION: CoreFusionStrategy = CoreFusionStrategy::RRF { k: 60 };

/// A vector collection in VelesDB.
///
/// Collections store vectors with optional metadata (payload) and support
/// efficient similarity search.
#[pyclass]
pub struct Collection {
    /// Core collection (cheap to clone — all fields are `Arc`-wrapped internally).
    pub(crate) inner: CoreCollection,
    /// Cached name to avoid acquiring `config` read lock on every `#[getter]` access.
    pub(crate) name: String,
}

impl Collection {
    /// Create a new Collection wrapper.
    pub fn new(inner: CoreCollection, name: String) -> Self {
        Self { inner, name }
    }

    /// Dispatch to the correct search path based on which arguments are present.
    ///
    /// Handles all combinations of dense/sparse with optional filter and
    /// optional named sparse index selection.
    fn dispatch_search(
        &self,
        dense: Option<Vec<f32>>,
        sparse: Option<velesdb_core::sparse_index::SparseVector>,
        top_k: usize,
        filter: Option<&Filter>,
        sparse_index_name: Option<&str>,
    ) -> PyResult<Vec<SearchResult>> {
        use crate::collection_helpers::core_err;
        use pyo3::exceptions::PyValueError;

        let index_name =
            sparse_index_name.unwrap_or(velesdb_core::sparse_index::DEFAULT_SPARSE_INDEX_NAME);

        match (dense, sparse, filter) {
            (Some(d), Some(s), Some(f)) => {
                // No native hybrid_sparse_search_with_filter — run unfiltered then post-filter
                let mut results = self
                    .inner
                    .hybrid_sparse_search(&d, &s, top_k, index_name, &DEFAULT_FUSION)
                    .map_err(core_err)?;
                results.retain(|r| {
                    r.point
                        .payload
                        .as_ref()
                        .is_some_and(|p| f.matches(p))
                });
                Ok(results)
            }
            (Some(d), Some(s), None) => self
                .inner
                .hybrid_sparse_search(&d, &s, top_k, index_name, &DEFAULT_FUSION)
                .map_err(core_err),
            (Some(d), None, Some(f)) => self
                .inner
                .search_with_filter(&d, top_k, f)
                .map_err(core_err),
            (Some(d), None, None) => self.inner.search(&d, top_k).map_err(core_err),
            (None, Some(_), Some(_)) => Err(PyValueError::new_err(
                "Filter is not supported with sparse-only search; provide 'vector' for hybrid search",
            )),
            (None, Some(s), None) => self
                .inner
                .sparse_search(&s, top_k, index_name)
                .map_err(core_err),
            (None, None, _) => Err(PyValueError::new_err(
                "At least one of 'vector' or 'sparse_vector' must be provided",
            )),
        }
    }
}

#[pymethods]
impl Collection {
    /// Get the collection name.
    #[getter]
    fn name(&self) -> &str {
        &self.name
    }

    /// Get collection configuration info.
    ///
    /// Returns:
    ///     Dict with name, dimension, metric, storage_mode, point_count, and metadata_only
    fn info(&self, py: Python<'_>) -> PyResult<PyObject> {
        let config = self.inner.config();
        let dict = PyDict::new(py);
        let _ = dict.set_item(PyString::intern(py, "name"), config.name.as_str());
        let _ = dict.set_item(PyString::intern(py, "dimension"), config.dimension);
        let _ = dict.set_item(
            PyString::intern(py, "metric"),
            format!("{:?}", config.metric).to_lowercase(),
        );
        let _ = dict.set_item(
            PyString::intern(py, "storage_mode"),
            format!("{:?}", config.storage_mode).to_lowercase(),
        );
        let _ = dict.set_item(PyString::intern(py, "point_count"), config.point_count);
        let _ = dict.set_item(PyString::intern(py, "metadata_only"), config.metadata_only);
        Ok(dict.into_any().unbind())
    }

    /// Check if this is a metadata-only collection.
    fn is_metadata_only(&self) -> bool {
        self.inner.is_metadata_only()
    }

    /// Check if the collection is empty.
    fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}
