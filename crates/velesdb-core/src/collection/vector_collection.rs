//! `VectorCollection`: newtype wrapper around `Collection` for vector workloads.
//!
//! This type provides a stable, typed API for vector collections.
//! Internally it delegates 100% to the `Collection` executor to avoid
//! any data synchronisation issues between separate storage layers.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::collection::types::{Collection, CollectionConfig};
use crate::distance::DistanceMetric;
use crate::error::Result;
use crate::point::{Point, SearchResult};
use crate::quantization::StorageMode;

/// A vector collection combining HNSW search, payload storage, and full-text search.
///
/// `VectorCollection` is a typed newtype over `Collection` that provides
/// a stable public API for vector workloads. All storage operations delegate
/// to the single `inner: Collection` instance — no dual-storage desync.
///
/// # Examples
///
/// ```rust,no_run
/// use velesdb_core::{VectorCollection, DistanceMetric, Point, StorageMode};
/// use serde_json::json;
///
/// let coll = VectorCollection::create(
///     "./data/docs".into(),
///     "docs",
///     768,
///     DistanceMetric::Cosine,
///     StorageMode::Full,
/// )?;
///
/// coll.upsert(vec![
///     Point::new(1, vec![0.1; 768], Some(json!({"title": "Hello"}))),
/// ])?;
///
/// let results = coll.search(&vec![0.1; 768], 10)?;
/// # Ok::<(), velesdb_core::Error>(())
/// ```
#[derive(Clone)]
pub struct VectorCollection {
    /// Single source of truth — all operations delegate here.
    pub(crate) inner: Collection,
}

impl VectorCollection {
    // -------------------------------------------------------------------------
    // Lifecycle
    // -------------------------------------------------------------------------

    /// Creates a new `VectorCollection` at the given path.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created or storage fails.
    pub fn create(
        path: PathBuf,
        _name: &str,
        dimension: usize,
        metric: DistanceMetric,
        storage_mode: StorageMode,
    ) -> Result<Self> {
        Ok(Self {
            inner: Collection::create_with_options(path, dimension, metric, storage_mode)?,
        })
    }

    /// Creates a new `VectorCollection` with custom HNSW parameters.
    ///
    /// When `m` or `ef_construction` are `Some`, those values override the
    /// auto-tuned defaults. When both are `None`, this is equivalent to
    /// [`VectorCollection::create`].
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created or storage fails.
    pub fn create_with_hnsw(
        path: PathBuf,
        _name: &str,
        dimension: usize,
        metric: DistanceMetric,
        storage_mode: StorageMode,
        m: Option<usize>,
        ef_construction: Option<usize>,
    ) -> Result<Self> {
        let mut params = crate::index::hnsw::HnswParams::auto(dimension);
        if let Some(m) = m {
            params.max_connections = m;
        }
        if let Some(ef) = ef_construction {
            params.ef_construction = ef;
        }
        params.storage_mode = storage_mode;
        Ok(Self {
            inner: Collection::create_with_hnsw_params(
                path,
                dimension,
                metric,
                storage_mode,
                params,
            )?,
        })
    }

    /// Opens an existing `VectorCollection` from disk.
    ///
    /// # Errors
    ///
    /// Returns an error if the config file cannot be read or storage cannot be opened.
    pub fn open(path: PathBuf) -> Result<Self> {
        Ok(Self {
            inner: Collection::open(path)?,
        })
    }

    /// Flushes all engines to disk and saves the config.
    ///
    /// # Errors
    ///
    /// Returns an error if any flush operation fails.
    pub fn flush(&self) -> Result<()> {
        self.inner.flush()
    }

    // -------------------------------------------------------------------------
    // Collection metadata — all delegate to inner
    // -------------------------------------------------------------------------

    /// Returns the collection name.
    #[must_use]
    pub fn name(&self) -> String {
        self.inner.config().name
    }

    /// Returns the vector dimension.
    #[must_use]
    pub fn dimension(&self) -> usize {
        self.inner.config().dimension
    }

    /// Returns the distance metric.
    #[must_use]
    pub fn metric(&self) -> DistanceMetric {
        self.inner.config().metric
    }

    /// Returns the storage mode.
    #[must_use]
    pub fn storage_mode(&self) -> StorageMode {
        self.inner.config().storage_mode
    }

    /// Returns the number of points in the collection.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Returns `true` if the collection is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Returns all point IDs.
    #[must_use]
    pub fn all_ids(&self) -> Vec<u64> {
        self.inner.all_ids()
    }

    /// Returns the current collection config.
    #[must_use]
    pub fn config(&self) -> CollectionConfig {
        self.inner.config()
    }

    // -------------------------------------------------------------------------
    // CRUD — all delegate to inner
    // -------------------------------------------------------------------------

    /// Bulk insert optimized for high-throughput import.
    ///
    /// # Errors
    ///
    /// Returns an error if any point has a mismatched dimension.
    pub fn upsert_bulk(&self, points: &[Point]) -> Result<usize> {
        self.inner.upsert_bulk(points)
    }

    /// Inserts or updates points.
    ///
    /// # Errors
    ///
    /// Returns an error if the dimension mismatches or storage fails.
    pub fn upsert(&self, points: impl IntoIterator<Item = Point>) -> Result<()> {
        self.inner.upsert(points)
    }

    /// Retrieves points by IDs.
    #[must_use]
    pub fn get(&self, ids: &[u64]) -> Vec<Option<Point>> {
        self.inner.get(ids)
    }

    /// Deletes points by IDs.
    ///
    /// # Errors
    ///
    /// Returns an error if storage operations fail.
    pub fn delete(&self, ids: &[u64]) -> Result<()> {
        self.inner.delete(ids)
    }

    // -------------------------------------------------------------------------
    // Search — all delegate to inner
    // -------------------------------------------------------------------------

    /// Performs kNN vector search.
    /// # Errors
    pub fn search(&self, query: &[f32], k: usize) -> Result<Vec<SearchResult>> {
        self.inner.search(query, k)
    }

    /// Performs full-text BM25 search.
    #[must_use]
    pub fn text_search(&self, query: &str, k: usize) -> Vec<SearchResult> {
        self.inner.text_search(query, k)
    }

    /// kNN search with explicit ef_search override.
    /// # Errors
    pub fn search_with_ef(
        &self,
        query: &[f32],
        k: usize,
        ef_search: usize,
    ) -> Result<Vec<SearchResult>> {
        self.inner.search_with_ef(query, k, ef_search)
    }

    /// kNN search with metadata filter.
    /// # Errors
    pub fn search_with_filter(
        &self,
        query: &[f32],
        k: usize,
        filter: &crate::filter::Filter,
    ) -> Result<Vec<SearchResult>> {
        self.inner.search_with_filter(query, k, filter)
    }

    /// Returns `(id, score)` pairs without payload hydration.
    /// # Errors
    pub fn search_ids(&self, query: &[f32], k: usize) -> Result<Vec<(u64, f32)>> {
        self.inner.search_ids(query, k)
    }

    /// Full-text search with metadata filter.
    #[must_use]
    pub fn text_search_with_filter(
        &self,
        query: &str,
        k: usize,
        filter: &crate::filter::Filter,
    ) -> Vec<SearchResult> {
        self.inner.text_search_with_filter(query, k, filter)
    }

    /// Hybrid search (vector + BM25 with RRF fusion).
    /// # Errors
    pub fn hybrid_search(
        &self,
        vector: &[f32],
        text: &str,
        k: usize,
        alpha: Option<f32>,
    ) -> Result<Vec<SearchResult>> {
        self.inner.hybrid_search(vector, text, k, alpha)
    }

    /// Hybrid search with metadata filter.
    /// # Errors
    pub fn hybrid_search_with_filter(
        &self,
        vector: &[f32],
        text: &str,
        k: usize,
        alpha: Option<f32>,
        filter: &crate::filter::Filter,
    ) -> Result<Vec<SearchResult>> {
        self.inner
            .hybrid_search_with_filter(vector, text, k, alpha, filter)
    }

    /// Batch search with per-query filters.
    /// # Errors
    pub fn search_batch_with_filters(
        &self,
        queries: &[&[f32]],
        k: usize,
        filters: &[Option<crate::filter::Filter>],
    ) -> Result<Vec<Vec<SearchResult>>> {
        self.inner.search_batch_with_filters(queries, k, filters)
    }

    /// Multi-query search (multiple vectors fused).
    /// # Errors
    pub fn multi_query_search(
        &self,
        queries: &[&[f32]],
        k: usize,
        strategy: crate::fusion::FusionStrategy,
        filter: Option<&crate::filter::Filter>,
    ) -> Result<Vec<SearchResult>> {
        self.inner.multi_query_search(queries, k, strategy, filter)
    }

    /// Multi-query search returning only IDs and fused scores.
    /// # Errors
    pub fn multi_query_search_ids(
        &self,
        queries: &[&[f32]],
        k: usize,
        strategy: crate::fusion::FusionStrategy,
    ) -> Result<Vec<(u64, f32)>> {
        self.inner.multi_query_search_ids(queries, k, strategy)
    }

    // -------------------------------------------------------------------------
    // Data mutations (metadata)
    // -------------------------------------------------------------------------

    /// Inserts or updates metadata-only points (no vectors).
    /// # Errors
    pub fn upsert_metadata(
        &self,
        points: impl IntoIterator<Item = crate::point::Point>,
    ) -> Result<()> {
        self.inner.upsert_metadata(points)
    }

    // -------------------------------------------------------------------------
    // Index management
    // -------------------------------------------------------------------------

    /// Creates a secondary metadata index on a payload field.
    /// # Errors
    pub fn create_index(&self, field: &str) -> Result<()> {
        self.inner.create_index(field)
    }

    /// Returns `true` if a secondary index exists on `field`.
    #[must_use]
    pub fn has_secondary_index(&self, field: &str) -> bool {
        self.inner.has_secondary_index(field)
    }

    /// Creates a property index for O(1) equality lookups.
    /// # Errors
    pub fn create_property_index(&self, label: &str, property: &str) -> Result<()> {
        self.inner.create_property_index(label, property)
    }

    /// Creates a range index for O(log n) range queries.
    /// # Errors
    pub fn create_range_index(&self, label: &str, property: &str) -> Result<()> {
        self.inner.create_range_index(label, property)
    }

    /// Returns `true` if a property index exists.
    #[must_use]
    pub fn has_property_index(&self, label: &str, property: &str) -> bool {
        self.inner.has_property_index(label, property)
    }

    /// Returns `true` if a range index exists.
    #[must_use]
    pub fn has_range_index(&self, label: &str, property: &str) -> bool {
        self.inner.has_range_index(label, property)
    }

    /// Lists all index definitions on this collection.
    #[must_use]
    pub fn list_indexes(&self) -> Vec<crate::collection::IndexInfo> {
        self.inner.list_indexes()
    }

    /// Drops an index. Returns `true` if an index was removed.
    /// # Errors
    pub fn drop_index(&self, label: &str, property: &str) -> Result<bool> {
        self.inner.drop_index(label, property)
    }

    /// Returns total memory usage of all indexes in bytes.
    #[must_use]
    pub fn indexes_memory_usage(&self) -> usize {
        self.inner.indexes_memory_usage()
    }

    // -------------------------------------------------------------------------
    // Match (graph pattern)
    // -------------------------------------------------------------------------

    /// Executes a graph MATCH query.
    /// # Errors
    pub fn execute_match(
        &self,
        match_clause: &crate::velesql::MatchClause,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> crate::error::Result<Vec<crate::collection::search::query::match_exec::MatchResult>> {
        self.inner.execute_match(match_clause, params)
    }

    /// Executes a MATCH query with vector similarity filtering.
    /// # Errors
    pub fn execute_match_with_similarity(
        &self,
        match_clause: &crate::velesql::MatchClause,
        query_vector: &[f32],
        threshold: f32,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> crate::error::Result<Vec<crate::collection::search::query::match_exec::MatchResult>> {
        self.inner
            .execute_match_with_similarity(match_clause, query_vector, threshold, params)
    }

    // -------------------------------------------------------------------------
    // Aggregation
    // -------------------------------------------------------------------------

    /// Executes an aggregation query (GROUP BY / COUNT / SUM / AVG / MIN / MAX).
    /// # Errors
    pub fn execute_aggregate(
        &self,
        query: &crate::velesql::Query,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<serde_json::Value> {
        self.inner.execute_aggregate(query, params)
    }

    // -------------------------------------------------------------------------
    // Statistics / misc
    // -------------------------------------------------------------------------

    /// Returns CBO statistics.
    #[must_use]
    pub fn get_stats(&self) -> crate::collection::stats::CollectionStats {
        self.inner.get_stats()
    }

    /// Returns `true` if the collection is a metadata-only collection.
    #[must_use]
    pub fn is_metadata_only(&self) -> bool {
        self.inner.is_metadata_only()
    }

    /// Analyzes the collection and returns fresh statistics.
    /// # Errors
    pub fn analyze(&self) -> Result<crate::collection::stats::CollectionStats> {
        self.inner.analyze()
    }

    // -------------------------------------------------------------------------
    // Sparse search
    // -------------------------------------------------------------------------

    /// Performs sparse-only search on the named index.
    ///
    /// # Errors
    ///
    /// Returns an error if the named sparse index does not exist.
    pub fn sparse_search(
        &self,
        query: &crate::index::sparse::SparseVector,
        k: usize,
        index_name: &str,
    ) -> Result<Vec<SearchResult>> {
        let indexes = self.inner.sparse_indexes.read();
        let index = indexes.get(index_name).ok_or_else(|| {
            crate::error::Error::Config(format!(
                "Sparse index '{}' not found",
                if index_name.is_empty() {
                    "<default>"
                } else {
                    index_name
                }
            ))
        })?;
        let results = crate::index::sparse::sparse_search(index, query, k);
        drop(indexes);
        Ok(self.inner.resolve_sparse_results(&results, k))
    }

    /// Performs hybrid dense+sparse search with RRF fusion.
    ///
    /// # Errors
    ///
    /// Returns an error if dense or sparse search fails, or fusion errors.
    #[allow(clippy::too_many_arguments)]
    pub fn hybrid_sparse_search(
        &self,
        dense_vector: &[f32],
        sparse_query: &crate::index::sparse::SparseVector,
        k: usize,
        index_name: &str,
        strategy: &crate::fusion::FusionStrategy,
    ) -> Result<Vec<SearchResult>> {
        let candidate_k = k.saturating_mul(2).max(k + 10);

        let (dense_results, sparse_results) = self.inner.execute_both_branches(
            dense_vector,
            sparse_query,
            index_name,
            candidate_k,
            None,
        );

        if dense_results.is_empty() && sparse_results.is_empty() {
            return Ok(Vec::new());
        }
        if dense_results.is_empty() {
            let scored: Vec<(u64, f32)> = sparse_results
                .iter()
                .map(|sd| (sd.doc_id, sd.score))
                .collect();
            return Ok(self.inner.resolve_fused_results(&scored, k));
        }
        if sparse_results.is_empty() {
            return Ok(self.inner.resolve_fused_results(&dense_results, k));
        }

        let sparse_tuples: Vec<(u64, f32)> = sparse_results
            .iter()
            .map(|sd| (sd.doc_id, sd.score))
            .collect();

        let fused = strategy
            .fuse(vec![dense_results, sparse_tuples])
            .map_err(|e| crate::error::Error::Config(format!("Fusion error: {e}")))?;

        Ok(self.inner.resolve_fused_results(&fused, k))
    }

    // -------------------------------------------------------------------------
    // VelesQL
    // -------------------------------------------------------------------------

    /// Executes a `VelesQL` query.
    /// # Errors
    pub fn execute_query(
        &self,
        query: &crate::velesql::Query,
        params: &HashMap<String, serde_json::Value>,
    ) -> Result<Vec<SearchResult>> {
        self.inner.execute_query(query, params)
    }

    /// Sends a point into the streaming ingestion channel.
    ///
    /// Returns `Ok(())` on success (202 semantics). Returns
    /// `BackpressureError::BufferFull` when the channel is at capacity, or
    /// `BackpressureError::NotConfigured` if streaming is not active.
    ///
    /// # Errors
    ///
    /// Returns `BackpressureError` on buffer-full or not-configured.
    #[cfg(feature = "persistence")]
    pub fn stream_insert(
        &self,
        point: crate::point::Point,
    ) -> std::result::Result<(), crate::collection::streaming::BackpressureError> {
        self.inner.stream_insert(point)
    }

    /// Pushes `(id, vector)` entries into the delta buffer if it is active.
    ///
    /// No-op when the delta buffer is inactive. This is the public interface
    /// used by streaming upsert handlers (e.g., NDJSON stream endpoint) to
    /// keep the delta buffer in sync after a successful `upsert_bulk` call.
    #[cfg(feature = "persistence")]
    pub fn push_to_delta_if_active(&self, entries: &[(u64, Vec<f32>)]) {
        self.inner.push_to_delta_if_active(entries);
    }

    /// Returns `true` if the delta buffer is currently active (HNSW rebuild
    /// in progress). External callers can use this to decide whether to
    /// snapshot entries for delta before a `upsert_bulk` call.
    #[cfg(feature = "persistence")]
    #[must_use]
    pub fn is_delta_active(&self) -> bool {
        self.inner.delta_buffer.is_active()
    }

    /// Executes a raw VelesQL string.
    /// # Errors
    pub fn execute_query_str(
        &self,
        sql: &str,
        params: &HashMap<String, serde_json::Value>,
    ) -> Result<Vec<SearchResult>> {
        let query = self
            .inner
            .query_cache
            .parse(sql)
            .map_err(|e| crate::error::Error::Query(e.to_string()))?;
        self.inner.execute_query(&query, params)
    }
}
