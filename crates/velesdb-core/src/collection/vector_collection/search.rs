//! Search, match, aggregation, and query execution for `VectorCollection`.

use std::collections::HashMap;

use crate::error::Result;
use crate::point::SearchResult;

use super::VectorCollection;

impl VectorCollection {
    /// Performs kNN vector search using the HNSW index.
    ///
    /// Returns the `k` nearest neighbors ordered by ascending distance.
    ///
    /// # Errors
    ///
    /// - Returns an error if the query dimension does not match the collection.
    /// - Returns an error if the HNSW index is not initialized.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # use velesdb_core::{VectorCollection, DistanceMetric, StorageMode};
    /// # let coll = VectorCollection::create("./data/v".into(), "v", 128, DistanceMetric::Cosine, StorageMode::Full)?;
    /// let results = coll.search(&vec![0.1; 128], 10)?;
    /// for r in &results {
    ///     println!("id={} score={}", r.point.id, r.score);
    /// }
    /// # Ok::<(), velesdb_core::Error>(())
    /// ```
    pub fn search(&self, query: &[f32], k: usize) -> Result<Vec<SearchResult>> {
        self.inner.search(query, k)
    }

    /// Performs full-text BM25 search over indexed payload fields.
    ///
    /// Returns up to `k` results ranked by BM25 relevance score.
    ///
    /// # Errors
    ///
    /// - Returns an error if storage retrieval fails.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # use velesdb_core::{VectorCollection, DistanceMetric, StorageMode};
    /// # let coll = VectorCollection::create("./data/v".into(), "v", 128, DistanceMetric::Cosine, StorageMode::Full)?;
    /// let results = coll.text_search("machine learning", 5)?;
    /// # Ok::<(), velesdb_core::Error>(())
    /// ```
    pub fn text_search(&self, query: &str, k: usize) -> Result<Vec<SearchResult>> {
        self.inner.text_search(query, k)
    }

    /// Performs kNN search with an explicit `ef_search` override.
    ///
    /// Higher `ef_search` values improve recall at the cost of latency.
    ///
    /// # Errors
    ///
    /// - Returns an error if the query dimension does not match the collection.
    pub fn search_with_ef(
        &self,
        query: &[f32],
        k: usize,
        ef_search: usize,
    ) -> Result<Vec<SearchResult>> {
        self.inner.search_with_ef(query, k, ef_search)
    }

    /// Performs kNN search with a metadata filter applied post-retrieval.
    ///
    /// # Errors
    ///
    /// - Returns an error if the query dimension does not match the collection.
    /// - Returns an error if the filter references an unsupported field type.
    pub fn search_with_filter(
        &self,
        query: &[f32],
        k: usize,
        filter: &crate::filter::Filter,
    ) -> Result<Vec<SearchResult>> {
        self.inner.search_with_filter(query, k, filter)
    }

    /// Returns [`ScoredResult`] pairs without payload hydration.
    ///
    /// Faster than [`search`](Self::search) when only IDs and scores are needed.
    ///
    /// # Errors
    ///
    /// - Returns an error if the query dimension does not match the collection.
    pub fn search_ids(
        &self,
        query: &[f32],
        k: usize,
    ) -> Result<Vec<crate::scored_result::ScoredResult>> {
        self.inner.search_ids(query, k)
    }

    /// Full-text search with metadata filter.
    ///
    /// # Errors
    ///
    /// Returns an error if storage retrieval fails.
    pub fn text_search_with_filter(
        &self,
        query: &str,
        k: usize,
        filter: &crate::filter::Filter,
    ) -> Result<Vec<SearchResult>> {
        self.inner.text_search_with_filter(query, k, filter)
    }

    /// Performs hybrid search combining vector kNN and BM25 full-text via RRF fusion.
    ///
    /// When `alpha` is `None`, a default blending factor is used. Values closer
    /// to `1.0` weight vector results more; values closer to `0.0` weight text.
    ///
    /// # Errors
    ///
    /// - Returns an error if the query dimension does not match the collection.
    /// - Returns an error if text indexing or storage retrieval fails.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # use velesdb_core::{VectorCollection, DistanceMetric, StorageMode};
    /// # let coll = VectorCollection::create("./data/v".into(), "v", 128, DistanceMetric::Cosine, StorageMode::Full)?;
    /// let results = coll.hybrid_search(&vec![0.1; 128], "machine learning", 10, Some(0.7))?;
    /// # Ok::<(), velesdb_core::Error>(())
    /// ```
    pub fn hybrid_search(
        &self,
        vector: &[f32],
        text: &str,
        k: usize,
        alpha: Option<f32>,
    ) -> Result<Vec<SearchResult>> {
        self.inner.hybrid_search(vector, text, k, alpha)
    }

    /// Performs hybrid search (vector + BM25) with a metadata filter.
    ///
    /// # Errors
    ///
    /// - Returns an error if the query dimension does not match the collection.
    /// - Returns an error if text indexing, storage, or filtering fails.
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

    /// Performs batch kNN search with per-query metadata filters.
    ///
    /// Each query in `queries` is paired with the filter at the same index in
    /// `filters`. Pass `None` for queries that should not be filtered.
    ///
    /// # Errors
    ///
    /// - Returns an error if any query dimension does not match the collection.
    /// - Returns an error if `queries` and `filters` have different lengths.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # use velesdb_core::{VectorCollection, DistanceMetric, StorageMode};
    /// # let coll = VectorCollection::create("./data/v".into(), "v", 128, DistanceMetric::Cosine, StorageMode::Full)?;
    /// let q1 = vec![0.1; 128];
    /// let q2 = vec![0.2; 128];
    /// let results = coll.search_batch_with_filters(
    ///     &[q1.as_slice(), q2.as_slice()],
    ///     10,
    ///     &[None, None],
    /// )?;
    /// assert_eq!(results.len(), 2);
    /// # Ok::<(), velesdb_core::Error>(())
    /// ```
    pub fn search_batch_with_filters(
        &self,
        queries: &[&[f32]],
        k: usize,
        filters: &[Option<crate::filter::Filter>],
    ) -> Result<Vec<Vec<SearchResult>>> {
        self.inner.search_batch_with_filters(queries, k, filters)
    }

    /// Performs multi-query search fusing results from multiple query vectors.
    ///
    /// # Errors
    ///
    /// - Returns an error if any query dimension does not match the collection.
    /// - Returns an error if the fusion strategy fails.
    pub fn multi_query_search(
        &self,
        queries: &[&[f32]],
        k: usize,
        strategy: crate::fusion::FusionStrategy,
        filter: Option<&crate::filter::Filter>,
    ) -> Result<Vec<SearchResult>> {
        self.inner.multi_query_search(queries, k, strategy, filter)
    }

    /// Performs multi-query search returning only IDs and fused scores.
    ///
    /// # Errors
    ///
    /// - Returns an error if any query dimension does not match the collection.
    /// - Returns an error if the fusion strategy fails.
    pub fn multi_query_search_ids(
        &self,
        queries: &[&[f32]],
        k: usize,
        strategy: crate::fusion::FusionStrategy,
    ) -> Result<Vec<(u64, f32)>> {
        self.inner.multi_query_search_ids(queries, k, strategy)
    }

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

    /// Executes a graph MATCH query against the collection's edge store.
    ///
    /// # Errors
    ///
    /// - Returns an error if the match clause references an invalid label or property.
    /// - Returns an error if the edge store is not initialized.
    pub fn execute_match(
        &self,
        match_clause: &crate::velesql::MatchClause,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> crate::error::Result<Vec<crate::collection::search::query::match_exec::MatchResult>> {
        self.inner.execute_match(match_clause, params)
    }

    /// Executes a MATCH query with vector similarity filtering.
    ///
    /// # Errors
    ///
    /// - Returns an error if the match clause is invalid or the query dimension mismatches.
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

    /// Executes an aggregation query (GROUP BY / COUNT / SUM / AVG / MIN / MAX).
    ///
    /// # Errors
    ///
    /// - Returns an error if the query is invalid or aggregation computation fails.
    pub fn execute_aggregate(
        &self,
        query: &crate::velesql::Query,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<serde_json::Value> {
        self.inner.execute_aggregate(query, params)
    }

    /// Executes a parsed `VelesQL` query.
    ///
    /// # Errors
    ///
    /// - Returns an error if the query references missing fields or execution fails.
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

    /// Executes a raw VelesQL string, parsing it before execution.
    ///
    /// # Errors
    ///
    /// - Returns an error if the SQL string cannot be parsed.
    /// - Returns an error if query execution fails.
    pub fn execute_query_str(
        &self,
        sql: &str,
        params: &HashMap<String, serde_json::Value>,
    ) -> Result<Vec<SearchResult>> {
        self.inner.execute_query_str(sql, params)
    }
}
