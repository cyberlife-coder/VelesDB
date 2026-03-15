//! Collection module for VelesDB Python bindings.
//!
//! Split into focused sub-modules:
//! - `search` — search methods (dense, sparse, hybrid, batch, multi-query)
//! - `query` — VelesQL query/match/explain methods
//! - `mutation` — upsert, delete, flush, stream_insert
//! - `index` — index CRUD (property, range)
//!
//! Note: Multiple `#[pymethods]` impl blocks across sub-modules are intentional.
//! PyO3 ≥ 0.21 supports this natively via inventory-based method registration.
//! rust-analyzer may incorrectly flag `PyMethods` trait conflicts — verify with `cargo build`.

mod index;
mod mutation;
mod query;
mod search;

use pyo3::prelude::*;
use std::collections::HashMap;
use std::sync::Arc;
use velesdb_core::{
    Collection as CoreCollection, Filter, FusionStrategy as CoreFusionStrategy, SearchResult,
};

/// A vector collection in VelesDB.
///
/// Collections store vectors with optional metadata (payload) and support
/// efficient similarity search.
#[pyclass]
pub struct Collection {
    pub(crate) inner: Arc<CoreCollection>,
    pub(crate) name: String,
}

impl Collection {
    /// Create a new Collection wrapper.
    pub fn new(inner: Arc<CoreCollection>, name: String) -> Self {
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
        use pyo3::exceptions::{PyRuntimeError, PyValueError};

        let index_name =
            sparse_index_name.unwrap_or(velesdb_core::sparse_index::DEFAULT_SPARSE_INDEX_NAME);

        match (dense, sparse, filter) {
            (Some(d), Some(s), Some(f)) => {
                let strategy = CoreFusionStrategy::RRF { k: 60 };
                self.inner
                    .hybrid_sparse_search_named_with_filter(
                        &d, &s, top_k, &strategy, index_name, f,
                    )
                    .map_err(|e| PyRuntimeError::new_err(format!("Hybrid search failed: {e}")))
            }
            (Some(d), Some(s), None) => {
                let strategy = CoreFusionStrategy::RRF { k: 60 };
                self.inner
                    .hybrid_sparse_search_named(&d, &s, top_k, &strategy, index_name)
                    .map_err(|e| PyRuntimeError::new_err(format!("Hybrid search failed: {e}")))
            }
            (Some(d), None, Some(f)) => self
                .inner
                .search_with_filter(&d, top_k, f)
                .map_err(|e| PyRuntimeError::new_err(format!("Search failed: {e}"))),
            (Some(d), None, None) => self
                .inner
                .search(&d, top_k)
                .map_err(|e| PyRuntimeError::new_err(format!("Search failed: {e}"))),
            (None, Some(_), Some(_)) => Err(PyValueError::new_err(
                "Filter is not supported with sparse-only search; provide 'vector' for hybrid search",
            )),
            (None, Some(s), None) => self
                .inner
                .sparse_search_named(&s, top_k, index_name)
                .map_err(|e| PyRuntimeError::new_err(format!("Sparse search failed: {e}"))),
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
    fn info(&self) -> PyResult<HashMap<String, PyObject>> {
        Python::with_gil(|py| {
            let config = self.inner.config();
            let mut info = HashMap::new();
            info.insert(
                "name".to_string(),
                crate::utils::to_pyobject(py, config.name.as_str()),
            );
            info.insert(
                "dimension".to_string(),
                crate::utils::to_pyobject(py, config.dimension),
            );
            info.insert(
                "metric".to_string(),
                crate::utils::to_pyobject(py, format!("{:?}", config.metric).to_lowercase()),
            );
            info.insert(
                "storage_mode".to_string(),
                crate::utils::to_pyobject(py, format!("{:?}", config.storage_mode).to_lowercase()),
            );
            info.insert(
                "point_count".to_string(),
                crate::utils::to_pyobject(py, config.point_count),
            );
            info.insert(
                "metadata_only".to_string(),
                crate::utils::to_pyobject(py, config.metadata_only),
            );
            Ok(info)
        })
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
