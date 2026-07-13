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

use std::sync::Arc;

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyString};
use velesdb_core::{
    Condition, Database as CoreDatabase, Filter, FusionStrategy as CoreFusionStrategy, GatedRead,
    QueryOperationKind, SearchResult, VectorCollection as CoreCollection,
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
/// Real kind of the core collection behind the single Python `Collection`
/// facade. Vector-only operations use it to fail loud on graph/metadata
/// collections instead of silently returning empty results (F2.2).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum CollectionKind {
    /// HNSW vector collection — supports vector search.
    Vector,
    /// Graph collection — edges and optional node embeddings.
    Graph,
    /// Metadata-only collection — payload/VelesQL, no vector search.
    Metadata,
}

impl CollectionKind {
    /// Human-readable label used in error messages.
    fn label(self) -> &'static str {
        match self {
            Self::Vector => "vector",
            Self::Graph => "graph",
            Self::Metadata => "metadata",
        }
    }
}

#[pyclass]
pub struct Collection {
    /// Core collection (cheap to clone — all fields are `Arc`-wrapped internally).
    pub(crate) inner: CoreCollection,
    /// Shared handle to the owning database. `inner` is a *detached* collection
    /// leaf with no observer reference, so every search is routed back through
    /// this handle's control-plane read gate (`gated_search` / `authorize_read`)
    /// rather than hitting the leaf directly — restoring governance for the
    /// Python direct-search API (OSS direct-search gate).
    pub(crate) db: Arc<CoreDatabase>,
    /// Cached name to avoid acquiring `config` read lock on every `#[getter]` access.
    pub(crate) name: String,
    /// Real kind of the wrapped collection; guards vector-only methods.
    pub(crate) kind: CollectionKind,
}

impl Collection {
    /// Create a wrapper for a vector collection.
    pub fn new(inner: CoreCollection, db: Arc<CoreDatabase>, name: String) -> Self {
        Self::new_with_kind(inner, db, name, CollectionKind::Vector)
    }

    /// Create a wrapper for a collection whose real kind may not be vector
    /// (graph/metadata), so vector-only methods fail loud rather than returning
    /// empty results (F2.2).
    pub(crate) fn new_with_kind(
        inner: CoreCollection,
        db: Arc<CoreDatabase>,
        name: String,
        kind: CollectionKind,
    ) -> Self {
        Self {
            inner,
            db,
            name,
            kind,
        }
    }

    /// Rejects a vector-only operation on a non-vector collection with a clear,
    /// actionable error instead of the silent empty result of F2.2.
    fn ensure_vector(&self) -> PyResult<()> {
        if self.kind == CollectionKind::Vector {
            return Ok(());
        }
        Err(pyo3::exceptions::PyValueError::new_err(format!(
            "vector search is not supported on the {} collection '{}'; \
             use execute_query() for VelesQL or the graph API instead",
            self.kind.label(),
            self.name
        )))
    }

    /// Dispatch to the correct search path based on which arguments are present.
    ///
    /// Every path is routed through the owning database's control-plane read
    /// gate first: dense-only / dense+filter reads map directly onto
    /// [`GatedRead::Dense`] and run via [`gated_search`](CoreDatabase::gated_search);
    /// sparse and hybrid-sparse reads have no `GatedRead` leaf, so they consult
    /// [`authorize_read`](CoreDatabase::authorize_read) and AND any observer
    /// scope filter into a post-filter (fail closed on deny, narrow on scope).
    #[allow(clippy::too_many_arguments)] // gate identity (principal/tenant) threads through
    fn dispatch_search(
        &self,
        dense: Option<Vec<f32>>,
        sparse: Option<velesdb_core::sparse_index::SparseVector>,
        top_k: usize,
        filter: Option<&Filter>,
        sparse_index_name: Option<&str>,
        fusion: Option<&CoreFusionStrategy>,
        principal: Option<&str>,
        tenant: Option<&str>,
    ) -> PyResult<Vec<SearchResult>> {
        use crate::collection_helpers::core_err;
        use pyo3::exceptions::PyValueError;

        // F2.2: vector search on a graph/metadata collection must fail loud,
        // not return an empty list.
        self.ensure_vector()?;

        let index_name =
            sparse_index_name.unwrap_or(velesdb_core::sparse_index::DEFAULT_SPARSE_INDEX_NAME);
        // Hybrid dense+sparse honors a caller-supplied fusion strategy
        // (backlog #24); other paths ignore it. Default preserves RRF k=60.
        let fusion = fusion.unwrap_or(&DEFAULT_FUSION);

        match (dense, sparse) {
            // Dense-only / dense+filter: expressible as a governed `GatedRead`.
            (Some(d), None) => self
                .db
                .gated_search(
                    &self.name,
                    principal,
                    tenant,
                    GatedRead::Dense {
                        query: &d,
                        k: top_k,
                        ef: None,
                        quality: None,
                        filter,
                    },
                )
                .map_err(core_err),
            // Any path carrying a sparse vector: the gate has no sparse leaf, so
            // authorize the read (deny ⇒ Err, fail closed) then AND the observer
            // scope filter into a post-filter so a scoped read only narrows.
            (dense_opt, Some(s)) => {
                let scope = self
                    .db
                    .authorize_read(
                        &self.name,
                        QueryOperationKind::VectorSearch,
                        principal,
                        tenant,
                    )
                    .map_err(core_err)?;
                self.run_sparse_gated(
                    dense_opt.as_deref(),
                    &s,
                    top_k,
                    filter,
                    scope.as_ref(),
                    index_name,
                    fusion,
                )
            }
            (None, None) => Err(PyValueError::new_err(
                "At least one of 'vector' or 'sparse_vector' must be provided",
            )),
        }
    }

    /// Runs a sparse-only or hybrid dense+sparse search after the read gate has
    /// authorized it, applying the caller filter and any observer scope filter
    /// via post-filtering (neither leaf accepts a metadata filter natively).
    #[allow(clippy::too_many_arguments)] // scope threading is intrinsic to the gate
    fn run_sparse_gated(
        &self,
        dense: Option<&[f32]>,
        sparse: &velesdb_core::sparse_index::SparseVector,
        top_k: usize,
        caller_filter: Option<&Filter>,
        scope_filter: Option<&Filter>,
        index_name: &str,
        fusion: &CoreFusionStrategy,
    ) -> PyResult<Vec<SearchResult>> {
        use crate::collection_helpers::core_err;
        use pyo3::exceptions::PyValueError;

        match dense {
            // Hybrid dense+sparse: no filtered leaf — run unfiltered then
            // post-filter by the caller filter AND the observer scope filter.
            Some(d) => {
                let mut results = self
                    .inner
                    .hybrid_sparse_search(d, sparse, top_k, index_name, fusion)
                    .map_err(core_err)?;
                retain_by_filters(&mut results, caller_filter, scope_filter);
                Ok(results)
            }
            // Sparse-only: a caller filter was never supported here (preserved
            // error). A governance scope filter still applies via post-filter,
            // so a scoped observer narrows rather than being silently bypassed.
            None => {
                if caller_filter.is_some() {
                    return Err(PyValueError::new_err(
                        "Filter is not supported with sparse-only search; provide 'vector' for hybrid search",
                    ));
                }
                let mut results = self
                    .inner
                    .sparse_search(sparse, top_k, index_name)
                    .map_err(core_err)?;
                retain_by_filters(&mut results, None, scope_filter);
                Ok(results)
            }
        }
    }

    /// Consults the read gate for a non-`GatedRead` search path (sparse, text on
    /// a non-vector collection, batch, multi-query). Returns the observer scope
    /// filter to AND into the query, `Ok(None)` to allow unmodified, or an
    /// `Err` when the read is denied (fail closed).
    ///
    /// `principal`/`tenant` are caller-supplied and only meaningful when a
    /// trusted embedder forwards a verified identity (local-SDK trust boundary):
    /// the gate authorizes against them but cannot itself authenticate the caller.
    fn authorize(
        &self,
        operation: QueryOperationKind,
        principal: Option<&str>,
        tenant: Option<&str>,
    ) -> PyResult<Option<Filter>> {
        self.db
            .authorize_read(&self.name, operation, principal, tenant)
            .map_err(crate::collection_helpers::core_err)
    }
}

/// AND-composes a caller filter with an observer scope filter. The result
/// matches only rows satisfying both, so composing a scope can only narrow.
fn and_scope(caller: Option<Filter>, scope: Option<Filter>) -> Option<Filter> {
    match (caller, scope) {
        (None, None) => None,
        (Some(c), None) => Some(c),
        (None, Some(s)) => Some(s),
        (Some(c), Some(s)) => Some(Filter::new(Condition::And {
            conditions: vec![c.condition, s.condition],
        })),
    }
}

/// Post-filters search results in place, dropping any row failing the caller
/// filter or the observer scope filter (either may be absent). A row with no
/// payload is dropped whenever a filter is active, matching the pre-existing
/// hybrid-sparse post-filter semantics.
///
/// Exposed `pub(crate)` so leaf-only search paths that take no metadata filter
/// (sparse, parallel batch, graph embedding search) can apply an observer scope
/// filter as a post-filter (narrow-only) after the gate authorizes the read.
pub(crate) fn retain_by_filters(
    results: &mut Vec<SearchResult>,
    caller: Option<&Filter>,
    scope: Option<&Filter>,
) {
    if caller.is_none() && scope.is_none() {
        return;
    }
    results.retain(|r| {
        r.point.payload.as_ref().is_some_and(|p| {
            caller.is_none_or(|f| f.matches(p)) && scope.is_none_or(|f| f.matches(p))
        })
    });
}

/// Refuses a search whose return shape (IDs and scores only, no payload) cannot
/// carry an observer scope filter. Rather than silently returning unscoped
/// rows, the read fails closed and the caller is pointed at a filterable entry
/// point. A `None` scope (plain allow) is a no-op.
fn deny_if_scoped(scope: Option<Filter>, context: &str) -> PyResult<()> {
    if scope.is_some() {
        return Err(pyo3::exceptions::PyPermissionError::new_err(format!(
            "{context} cannot honor the governance scope filter returned by the observer \
             (this entry point returns only ids/scores and has no metadata-filtered leaf); \
             refusing to run unscoped. Use search()/search_request() with the same query instead."
        )));
    }
    Ok(())
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
