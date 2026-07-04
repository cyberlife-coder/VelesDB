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
pub(crate) mod search_options;

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyString};
use velesdb_core::{
    Filter, FusionStrategy as CoreFusionStrategy, SearchResult, VectorCollection as CoreCollection,
};

/// Default fusion strategy when none is specified by the caller.
const DEFAULT_FUSION: CoreFusionStrategy = CoreFusionStrategy::RRF { k: 60 };

use velesdb_core::collection::streaming::{AsyncIndexBuilderConfig, DeferredIndexerConfig};

use crate::utils::opt_field;

/// Resolve a three-state override for `key`: an absent key leaves the field
/// unchanged (`None`), an explicit Python `None` clears it (`Some(None)`),
/// and any other value is built via `build` (`Some(Some(_))`).
fn three_state<T>(
    config: &Bound<'_, PyDict>,
    key: &str,
    build: impl Fn(&Bound<'_, PyAny>) -> PyResult<T>,
) -> PyResult<Option<Option<T>>> {
    match config.get_item(key)? {
        None => Ok(None),
        Some(v) if v.is_none() => Ok(Some(None)),
        Some(v) => Ok(Some(Some(build(&v)?))),
    }
}

/// Build a `DeferredIndexerConfig` from a Python dict, overriding only the
/// keys present on top of the struct defaults.
fn deferred_from_dict(value: &Bound<'_, PyAny>) -> PyResult<DeferredIndexerConfig> {
    let dict = value.cast::<PyDict>()?;
    let mut cfg = DeferredIndexerConfig::default();
    if let Some(v) = opt_field(dict, "enabled")? {
        cfg.enabled = v;
    }
    if let Some(v) = opt_field(dict, "merge_threshold")? {
        cfg.merge_threshold = v;
    }
    if let Some(v) = opt_field(dict, "max_buffer_age_ms")? {
        cfg.max_buffer_age_ms = v;
    }
    Ok(cfg)
}

/// Build an `AsyncIndexBuilderConfig` from a Python dict, overriding only the
/// keys present on top of the struct defaults. `segment_count` accepts a
/// Python `None` to fall back to the CPU count.
fn async_builder_from_dict(value: &Bound<'_, PyAny>) -> PyResult<AsyncIndexBuilderConfig> {
    let dict = value.cast::<PyDict>()?;
    let mut cfg = AsyncIndexBuilderConfig::default();
    if let Some(v) = opt_field(dict, "merge_threshold")? {
        cfg.merge_threshold = v;
    }
    if dict.get_item("segment_count")?.is_some() {
        cfg.segment_count = opt_field(dict, "segment_count")?;
    }
    Ok(cfg)
}

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
        fusion: Option<&CoreFusionStrategy>,
    ) -> PyResult<Vec<SearchResult>> {
        use crate::collection_helpers::core_err;
        use pyo3::exceptions::PyValueError;

        let index_name =
            sparse_index_name.unwrap_or(velesdb_core::sparse_index::DEFAULT_SPARSE_INDEX_NAME);
        // Hybrid dense+sparse honors a caller-supplied fusion strategy
        // (backlog #24); other paths ignore it. Default preserves RRF k=60.
        let fusion = fusion.unwrap_or(&DEFAULT_FUSION);

        match (dense, sparse, filter) {
            (Some(d), Some(s), Some(f)) => {
                // No native hybrid_sparse_search_with_filter — run unfiltered then post-filter
                let mut results = self
                    .inner
                    .hybrid_sparse_search(&d, &s, top_k, index_name, fusion)
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
                .hybrid_sparse_search(&d, &s, top_k, index_name, fusion)
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
    ///     Dict with name, dimension, metric, storage_mode, point_count,
    ///     metadata_only, and auto_reindex
    fn info(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let config = self.inner.config();
        let dict = PyDict::new(py);
        let _ = dict.set_item(PyString::intern(py, "name"), config.name.as_str());
        let _ = dict.set_item(PyString::intern(py, "dimension"), config.dimension);
        let _ = dict.set_item(
            PyString::intern(py, "metric"),
            config.metric.canonical_name(),
        );
        let _ = dict.set_item(
            PyString::intern(py, "storage_mode"),
            config.storage_mode.canonical_name(),
        );
        let _ = dict.set_item(PyString::intern(py, "point_count"), config.point_count);
        let _ = dict.set_item(PyString::intern(py, "metadata_only"), config.metadata_only);
        let auto_reindex = config
            .auto_reindex_config
            .as_ref()
            .is_some_and(|c| c.enabled);
        let _ = dict.set_item(PyString::intern(py, "auto_reindex"), auto_reindex);
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

    /// Get the vector dimension of this collection.
    ///
    /// Returns:
    ///     int: The dimension (e.g. 768 for BERT embeddings)
    #[getter]
    fn dimension(&self) -> usize {
        self.inner.dimension()
    }

    /// Get the distance metric used by this collection.
    ///
    /// Returns:
    ///     str: The canonical metric name (e.g. "cosine", "euclidean", "dot")
    #[getter]
    fn metric(&self) -> String {
        self.inner.metric().canonical_name().to_owned()
    }

    /// Get the storage mode of this collection.
    ///
    /// Returns:
    ///     str: The canonical storage mode name (e.g. "full", "sq8", "binary",
    ///          "pq", "rabitq").
    #[getter]
    fn storage_mode(&self) -> String {
        self.inner.storage_mode().canonical_name().to_owned()
    }

    /// Get the number of points in the collection.
    ///
    /// Returns:
    ///     int: The point count
    fn __len__(&self) -> usize {
        self.inner.len()
    }

    /// Get the number of points in the collection.
    ///
    /// Returns:
    ///     int: The point count
    fn count(&self) -> usize {
        self.inner.len()
    }

    /// Get all point IDs in the collection.
    ///
    /// Returns:
    ///     List[int]: All point IDs
    fn all_ids(&self, py: Python<'_>) -> Vec<u64> {
        py.detach(|| self.inner.all_ids())
    }

    /// Full durability flush including vectors.idx serialization.
    ///
    /// Use on graceful shutdown to avoid a full WAL replay on next startup.
    /// For routine persistence, use ``flush()`` instead.
    fn flush_full(&self, py: Python<'_>) -> PyResult<()> {
        use crate::collection_helpers::core_err;
        py.detach(|| self.inner.flush_full().map_err(core_err))
    }

    /// Compact on-disk storage, reclaiming space left by deleted vectors.
    ///
    /// Returns:
    ///     int: Number of bytes reclaimed.
    fn compact_storage(&self, py: Python<'_>) -> PyResult<usize> {
        use crate::collection_helpers::core_err;
        py.detach(|| self.inner.compact_storage().map_err(core_err))
    }

    /// Reorder the HNSW adjacency lists and vector storage for cache
    /// locality, so nodes traversed together during search sit close in
    /// memory. No-op for collections with fewer than 1000 vectors. Recall
    /// is preserved — only the physical layout changes.
    ///
    /// Best called after a bulk ``upsert`` on a freshly loaded collection,
    /// before serving queries.
    ///
    /// Returns:
    ///     None
    fn reorder_for_locality(&self, py: Python<'_>) -> PyResult<()> {
        use crate::collection_helpers::core_err;
        py.detach(|| self.inner.reorder_for_locality().map_err(core_err))
    }

    /// Apply post-creation overrides to advanced configuration fields and
    /// persist the updated ``config.json``.
    ///
    /// Three-state semantics per field: a key **absent** from ``config``
    /// leaves that field unchanged; a key present with value ``None``
    /// clears it; a key present with a value sets it.
    ///
    /// Args:
    ///     config: dict with optional keys ``pq_rescore_oversampling``
    ///         (int or None), ``deferred_indexing`` (dict with keys
    ///         ``enabled``, ``merge_threshold``, ``max_buffer_age_ms``, or
    ///         None), ``async_index_builder`` (dict with keys
    ///         ``merge_threshold``, ``segment_count``, or None).
    ///
    /// Returns:
    ///     None
    #[pyo3(signature = (config))]
    fn apply_advanced_config(&self, py: Python<'_>, config: &Bound<'_, PyDict>) -> PyResult<()> {
        use crate::collection_helpers::core_err;
        let pq = three_state(config, "pq_rescore_oversampling", |v| v.extract::<u32>())?;
        let deferred = three_state(config, "deferred_indexing", deferred_from_dict)?;
        let async_builder = three_state(config, "async_index_builder", async_builder_from_dict)?;
        py.detach(|| {
            self.inner
                .apply_advanced_config(pq, deferred, async_builder)
                .map_err(core_err)
        })
    }

    /// Get the current query guardrail limits for this collection.
    ///
    /// Returns:
    ///     dict with keys ``max_depth``, ``max_cardinality``,
    ///     ``memory_limit_bytes``, ``timeout_ms``, ``rate_limit_qps``,
    ///     ``circuit_failure_threshold``, ``circuit_recovery_seconds``.
    fn guard_rails(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let limits = self.inner.guard_rails().limits();
        let dict = PyDict::new(py);
        dict.set_item("max_depth", limits.max_depth)?;
        dict.set_item("max_cardinality", limits.max_cardinality)?;
        dict.set_item("memory_limit_bytes", limits.memory_limit_bytes)?;
        dict.set_item("timeout_ms", limits.timeout_ms)?;
        dict.set_item("rate_limit_qps", limits.rate_limit_qps)?;
        dict.set_item(
            "circuit_failure_threshold",
            limits.circuit_failure_threshold,
        )?;
        dict.set_item("circuit_recovery_seconds", limits.circuit_recovery_seconds)?;
        Ok(dict.into())
    }

    /// Detach the auto-reindex manager attached to this collection, if any.
    ///
    /// Returns:
    ///     bool: True if a manager was detached, False if none was attached.
    fn detach_auto_reindex(&self) -> bool {
        self.inner.detach_auto_reindex().is_some()
    }

    /// Check whether the attached auto-reindex manager considers the index
    /// diverged from its optimal parameters.
    ///
    /// Read-only — does not mutate the manager or trigger a reindex.
    ///
    /// Returns:
    ///     dict with keys ``should_reindex``, ``current_m``, ``optimal_m``,
    ///     ``ratio`` (and ``reason`` when a reindex is recommended), or
    ///     ``None`` when no auto-reindex manager is attached.
    fn check_auto_reindex_divergence(&self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        let Some(check) = self.inner.check_auto_reindex_divergence() else {
            return Ok(None);
        };
        let dict = PyDict::new(py);
        dict.set_item("should_reindex", check.should_reindex)?;
        dict.set_item("current_m", check.current_m)?;
        dict.set_item("optimal_m", check.optimal_m)?;
        dict.set_item("ratio", check.ratio)?;
        if let Some(reason) = &check.reason {
            dict.set_item("reason", format!("{reason:?}"))?;
        }
        Ok(Some(dict.into()))
    }

    /// Check if a secondary index exists on a payload field.
    ///
    /// Args:
    ///     field: The payload field name (e.g. "category")
    ///
    /// Returns:
    ///     bool: True if the index exists
    #[pyo3(signature = (field))]
    fn has_secondary_index(&self, field: &str) -> bool {
        self.inner.has_secondary_index(field)
    }

    /// Drop a secondary index on a payload field.
    ///
    /// Args:
    ///     field: The payload field name
    ///
    /// Returns:
    ///     bool: True if the index existed and was dropped
    #[pyo3(signature = (field))]
    fn drop_secondary_index(&self, field: &str) -> bool {
        self.inner.drop_secondary_index(field)
    }

    /// Get total memory usage of all indexes in bytes.
    ///
    /// Returns:
    ///     int: Memory usage in bytes
    fn indexes_memory_usage(&self) -> usize {
        self.inner.indexes_memory_usage()
    }

    /// Analyze the collection and compute fresh statistics.
    ///
    /// Returns:
    ///     dict: Statistics including row_count, deleted_count, total_size_bytes,
    ///           column_stats, index_stats, etc.
    fn analyze(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        use crate::collection_helpers::core_err;
        let stats = py.detach(|| self.inner.analyze().map_err(core_err))?;
        let json = serde_json::to_value(&stats).map_err(|e| {
            pyo3::exceptions::PyRuntimeError::new_err(format!("Stats serialization failed: {e}"))
        })?;
        Ok(crate::utils::json_to_python(py, &json))
    }

    /// Check if the streaming delta buffer is active (HNSW rebuild in progress).
    ///
    /// Returns:
    ///     bool: True if delta buffer is active
    fn is_delta_active(&self) -> bool {
        self.inner.is_delta_active()
    }

    /// Membership test: ``id in collection``.
    ///
    /// Args:
    ///     id: The point ID to look up
    ///
    /// Returns:
    ///     bool: True if a point with that ID exists in the collection
    ///
    /// Note: signature must omit ``py: Python<'_>`` so PyO3 installs this
    /// as the ``sq_contains`` slot. Otherwise Python falls back to
    /// ``__iter__`` and raises ``TypeError`` for the ``in`` operator.
    fn __contains__(&self, id: u64) -> PyResult<bool> {
        Ok(self.inner.get(&[id]).into_iter().next().flatten().is_some())
    }

    /// Graceful shutdown: full durability flush including ``vectors.idx``.
    ///
    /// Idempotent — safe to call multiple times. Equivalent to
    /// :py:meth:`flush_full` but named so collections can be used as a
    /// context manager (``with`` statement).
    fn close(&self, py: Python<'_>) -> PyResult<()> {
        use crate::collection_helpers::core_err;
        py.detach(|| self.inner.flush_full().map_err(core_err))
    }

    /// Context manager entry — returns ``self`` so the collection can be
    /// bound by the ``as`` clause in a ``with`` statement.
    fn __enter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    /// Context manager exit — calls :py:meth:`close` and re-raises any
    /// exception raised inside the ``with`` block.
    #[pyo3(signature = (_exc_type=None, _exc_value=None, _traceback=None))]
    fn __exit__(
        &self,
        py: Python<'_>,
        _exc_type: Option<Py<PyAny>>,
        _exc_value: Option<Py<PyAny>>,
        _traceback: Option<Py<PyAny>>,
    ) -> PyResult<bool> {
        self.close(py)?;
        Ok(false)
    }
}
