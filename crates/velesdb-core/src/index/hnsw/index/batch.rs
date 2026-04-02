//! Batch operations for HnswIndex.

use super::HnswIndex;
use crate::index::hnsw::params::SearchQuality;
use crate::index::hnsw::upsert::{self, UpsertResult};
use crate::scored_result::ScoredResult;
use crate::validation::validate_dimension_match;
use rayon::prelude::*;

/// Prepared batch of vectors ready for HNSW graph insertion.
///
/// Contains both the data needed for `parallel_insert` and the rollback
/// information needed to restore mappings on failure.
pub(crate) struct PreparedBatch<'a> {
    /// `(internal_idx, vector_slice)` pairs for HNSW insertion.
    pub to_insert: Vec<(usize, &'a [f32])>,
    /// `(external_id, upsert_result)` pairs for rollback on failure.
    pub rollback_info: Vec<(u64, UpsertResult)>,
}

impl HnswIndex {
    /// Prepares vectors for batch insertion: validates dimensions and registers IDs.
    ///
    /// Returns a [`PreparedBatch`] containing insertion data and rollback
    /// information. Existing IDs are replaced (upsert semantics): the old
    /// mapping is updated and stale vector data is removed.
    ///
    /// # Phase Ordering Invariant
    ///
    /// Dimension validation runs to completion **before** any call to
    /// `upsert_mapping_batch`. This prevents orphaned mappings when a
    /// dimension mismatch would otherwise leave partially-mutated
    /// state. See `docs/SOUNDNESS.md` "HNSW Batch Insertion Ordering".
    ///
    /// # Errors
    ///
    /// Returns [`Error::DimensionMismatch`] if any vector has a dimension
    /// different from the index dimension.
    ///
    /// # Performance
    ///
    /// - Single pass over input (no intermediate collection)
    /// - Pre-allocated output vectors
    /// - Inline dimension validation
    /// - Zero-copy: stores borrowed slices, no cloning
    #[inline]
    pub(crate) fn prepare_batch_insert<'a, I>(
        &self,
        vectors: I,
    ) -> crate::error::Result<PreparedBatch<'a>>
    where
        I: IntoIterator<Item = (u64, &'a [f32])>,
    {
        let items: Vec<(u64, &'a [f32])> = vectors.into_iter().collect();

        // Validate ALL dimensions upfront before any destructive upsert_mapping
        // calls. An error after partial upsert_mapping would leave orphaned mappings.
        for (_, vector) in &items {
            validate_dimension_match(self.dimension, vector.len())?;
        }

        let ids: Vec<u64> = items.iter().map(|(id, _)| *id).collect();
        let upsert_results = upsert::upsert_mapping_batch(
            &self.mappings,
            &self.vectors,
            self.enable_vector_storage,
            &ids,
        );

        let mut to_insert = Vec::with_capacity(items.len());
        let mut rollback_info = Vec::with_capacity(items.len());

        for ((id, vector), result) in items.into_iter().zip(upsert_results) {
            to_insert.push((result.idx, vector));
            rollback_info.push((id, result));
        }

        Ok(PreparedBatch {
            to_insert,
            rollback_info,
        })
    }

    /// Inserts multiple vectors in parallel using rayon.
    ///
    /// This method is optimized for bulk insertions and can significantly
    /// reduce indexing time on multi-core systems.
    ///
    /// # Ordering
    ///
    /// The pipeline executes: validate dims -> register mappings -> graph insert
    /// -> sidecar store. On graph failure, rollback undoes mappings in reverse
    /// order. Because `parallel_insert` uses rayon, the HNSW graph construction
    /// order is non-deterministic across runs (see v1.7.2 CHANGELOG note).
    ///
    /// # Arguments
    ///
    /// * `vectors` - Iterator of (id, vector) pairs to insert
    ///
    /// # Returns
    ///
    /// Number of vectors successfully inserted or updated (upsert semantics).
    ///
    /// # Panics
    ///
    /// Panics if any vector has a dimension different from the index dimension.
    ///
    /// # Performance (v0.8.5+)
    ///
    /// - **~15x faster** than sequential insertion (29k/s vs 1.9k/s on 8-core CPU)
    /// - Automatically scales with available CPU cores
    /// - Lock-free ID mapping via `DashMap`
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use velesdb_core::index::HnswIndex;
    /// use velesdb_core::DistanceMetric;
    ///
    /// let index = HnswIndex::new(128, DistanceMetric::Cosine);
    /// let vectors: Vec<_> = (0..1000)
    ///     .map(|i| (i as u64, vec![i as f32 / 1000.0; 128]))
    ///     .collect();
    ///
    /// let inserted = index.insert_batch_parallel(vectors);
    /// println!("Inserted {} vectors", inserted);
    /// ```
    pub fn insert_batch_parallel<'a, I>(&self, vectors: I) -> usize
    where
        I: IntoIterator<Item = (u64, &'a [f32])>,
    {
        let batch = match self.prepare_batch_insert(vectors) {
            Ok(b) => b,
            Err(e) => {
                tracing::error!("insert_batch_parallel: dimension validation failed: {e}");
                return 0;
            }
        };
        let count = batch.to_insert.len();

        if count == 0 {
            return 0;
        }

        let refs_for_hnsw: Vec<(&[f32], usize)> = batch
            .to_insert
            .iter()
            .map(|(idx, vec)| (*vec, *idx))
            .collect();

        // Insert into HNSW graph before storing vectors to avoid orphaned sidecar data.
        let assigned_ids = match self.inner.read().parallel_insert(&refs_for_hnsw) {
            Ok(ids) => ids,
            Err(e) => {
                tracing::error!("insert_batch_parallel: parallel_insert failed: {e}");
                // Reverse order: undo last upsert first so duplicate-ID chains
                // restore correctly (each rollback depends on the previous state).
                for (id, result) in batch.rollback_info.iter().rev() {
                    self.rollback_upsert(*id, result);
                }
                return 0;
            }
        };

        // Reconcile mappings: the graph may have assigned different node IDs
        // than the indices pre-registered by upsert_mapping_batch. Fix any
        // mismatches following the same pattern as insert_and_correct_mapping.
        let storage_ids =
            Self::reconcile_batch_mappings(&self.mappings, &batch.rollback_info, &assigned_ids);

        if self.enable_vector_storage {
            batch
                .to_insert
                .par_iter()
                .zip(storage_ids.par_iter())
                .for_each(|((_, vec), idx)| {
                    self.vectors.insert(*idx, vec);
                });
        }

        count
    }

    /// Reconciles pre-registered mapping indices with graph-assigned node IDs.
    ///
    /// For each item, if the graph-assigned ID differs from the pre-registered
    /// index, removes the stale reverse mapping and restores the correct one.
    /// Returns the authoritative storage index for each item.
    fn reconcile_batch_mappings(
        mappings: &crate::index::hnsw::sharded_mappings::ShardedMappings,
        rollback_info: &[(u64, UpsertResult)],
        assigned_ids: &[usize],
    ) -> Vec<usize> {
        let mut storage_ids = Vec::with_capacity(assigned_ids.len());
        for (assigned_id, (ext_id, result)) in assigned_ids.iter().zip(rollback_info) {
            if *assigned_id == result.idx {
                storage_ids.push(result.idx);
            } else {
                // Graph assigned a different node ID than upsert_mapping expected.
                // Remove the stale reverse mapping (result.idx -> ext_id) and
                // establish the correct mapping (ext_id <-> assigned_id).
                mappings.remove_reverse(result.idx);
                mappings.restore(*ext_id, *assigned_id);
                storage_ids.push(*assigned_id);
            }
        }
        storage_ids
    }

    /// Performs batch search for multiple queries in parallel.
    ///
    /// When quality requires two-stage reranking and vector storage is enabled,
    /// the method first runs HNSW search for all queries (rayon), then reranks
    /// each query's candidates using GPU or SIMD as appropriate. Otherwise,
    /// falls back to HNSW-only search.
    ///
    /// # Arguments
    ///
    /// * `queries` - Slice of query vectors (as slices)
    /// * `k` - Number of nearest neighbors per query
    /// * `quality` - Search quality profile
    ///
    /// # Returns
    ///
    /// Vector of results, one per query, each containing scored results.
    ///
    /// # Performance
    ///
    /// - Uses rayon for parallel HNSW search across all queries
    /// - GPU reranking batches all candidates per query for efficient dispatch
    /// - Falls back to SIMD reranking below GPU threshold
    ///
    /// # Errors
    ///
    /// Returns [`Error::DimensionMismatch`] if any query dimension does not
    /// match the index dimension.
    pub fn search_batch_parallel(
        &self,
        queries: &[&[f32]],
        k: usize,
        quality: SearchQuality,
    ) -> crate::error::Result<Vec<Vec<ScoredResult>>> {
        self.validate_batch_dimensions(queries)?;

        // Perfect, Adaptive, or very small collections: delegate to search_with_quality
        // per-query to match single-query behavior.
        // - Perfect: uses brute-force for 100% recall
        // - Adaptive: uses spread-based two-phase escalation (not batch-compatible)
        // - Small (<=100): uses brute-force for fully-connected graph safety
        if matches!(
            quality,
            SearchQuality::Perfect | SearchQuality::Adaptive { .. }
        ) || (self.len() <= 100 && self.enable_vector_storage && !self.vectors.is_empty())
        {
            let results: crate::error::Result<Vec<Vec<ScoredResult>>> = queries
                .par_iter()
                .map(|query| self.search_with_quality(query, k, quality))
                .collect();
            return results;
        }

        let ef_search = quality.ef_search(k);

        // Two-stage GPU/SIMD reranking for Balanced/Accurate/Custom qualities.
        if let Some(rerank_k) = self.should_two_stage_rerank(quality, k, ef_search) {
            return Ok(self.search_batch_with_rerank(queries, k, rerank_k, ef_search));
        }

        // Fast path: HNSW-only search for each query (rayon parallel)
        Ok(queries
            .par_iter()
            .map(|query| self.search_hnsw_only(query, k, ef_search))
            .collect())
    }

    /// Validates that all query vectors have the correct dimension.
    ///
    /// # Errors
    ///
    /// Returns [`Error::DimensionMismatch`] on the first query whose dimension
    /// does not match the index dimension.
    fn validate_batch_dimensions(&self, queries: &[&[f32]]) -> crate::error::Result<()> {
        for query in queries {
            validate_dimension_match(self.dimension, query.len())?;
        }
        Ok(())
    }

    /// Batch search with two-stage reranking for all queries.
    ///
    /// Phase 1: HNSW search with oversampled `rerank_k` candidates (rayon).
    /// Phase 2: Rerank each query's candidates via GPU or SIMD.
    fn search_batch_with_rerank(
        &self,
        queries: &[&[f32]],
        k: usize,
        rerank_k: usize,
        ef_search: usize,
    ) -> Vec<Vec<ScoredResult>> {
        let all_candidates: Vec<Vec<ScoredResult>> = queries
            .par_iter()
            .map(|query| self.search_hnsw_only(query, rerank_k, ef_search))
            .collect();

        // Rerank in parallel, collecting per-query latencies for aggregated EMA update.
        let timed_results: Vec<(Vec<ScoredResult>, u64)> = queries
            .par_iter()
            .zip(all_candidates.par_iter())
            .map(|(query, candidates)| self.rerank_sort_and_truncate_timed(query, candidates, k))
            .collect();

        let n = timed_results.len() as u64;
        if n > 0 {
            let total_us: u64 = timed_results.iter().map(|(_, e)| e).sum();
            let mean_us = total_us / n;
            if mean_us > 0 {
                self.update_rerank_latency_ema(mean_us);
            }
        }

        timed_results.into_iter().map(|(r, _)| r).collect()
    }

    /// Performs exact brute-force search in parallel using rayon.
    ///
    /// For large datasets (>100K vectors), automatically attempts GPU-accelerated
    /// search via `search_brute_force_gpu_inner` before falling back to rayon.
    ///
    /// # Arguments
    ///
    /// * `query` - The query vector
    /// * `k` - Number of nearest neighbors to return
    ///
    /// # Returns
    ///
    /// Vector of (id, score) tuples, sorted by similarity.
    ///
    /// # Performance
    ///
    /// - **Recall**: 100% (exact)
    /// - **Latency**: O(n/cores) on CPU, O(n/GPU-threads) on GPU
    /// - **GPU threshold**: 100K vectors (below this, rayon is faster)
    ///
    /// # Errors
    ///
    /// Returns [`Error::DimensionMismatch`] if the query dimension does not
    /// match the index dimension.
    pub fn brute_force_search_parallel(
        &self,
        query: &[f32],
        k: usize,
    ) -> crate::error::Result<Vec<ScoredResult>> {
        self.validate_dimension(query)?;

        // Try GPU path for large datasets where GPU upload overhead is amortized
        #[cfg(feature = "gpu")]
        if self.len() > Self::GPU_BRUTE_FORCE_THRESHOLD {
            if let Some(results) = self.search_brute_force_gpu_inner(query, k) {
                return Ok(results);
            }
        }

        Ok(self.brute_force_search_rayon(query, k))
    }

    /// GPU brute-force dispatch accessible from tests.
    ///
    /// Delegates to `search_brute_force_gpu_inner` without the 100K threshold
    /// gate, so tests can exercise the GPU path with smaller datasets.
    ///
    /// Returns `None` if GPU is unavailable.
    #[cfg(all(test, feature = "gpu"))]
    #[must_use]
    pub(crate) fn brute_force_search_gpu_dispatch(
        &self,
        query: &[f32],
        k: usize,
    ) -> Option<Vec<ScoredResult>> {
        self.search_brute_force_gpu_inner(query, k)
    }

    /// Rayon-based brute-force search over `ShardedVectors`.
    ///
    /// Extracted from `brute_force_search_parallel` so the GPU gate in that
    /// method stays compact. Also used by `search_brute_force` as the default
    /// compute path (RF-DEDUP: single parallel implementation).
    pub(super) fn brute_force_search_rayon(&self, query: &[f32], k: usize) -> Vec<ScoredResult> {
        let vectors_snapshot = self.vectors.collect_for_parallel();

        let mut results: Vec<ScoredResult> = vectors_snapshot
            .par_iter()
            .filter_map(|(idx, vec)| {
                let id = self.mappings.get_id(*idx)?;
                let score = self.compute_distance(query, vec);
                Some(ScoredResult::new(id, score))
            })
            .collect();

        self.metric.sort_scored_results(&mut results);

        results.truncate(k);
        results
    }

    /// Minimum dataset size for GPU brute-force dispatch.
    ///
    /// Benchmarks show wgpu has ~900 us of fixed overhead per dispatch.
    /// Below 100K vectors, rayon parallel SIMD is faster due to zero
    /// GPU buffer upload overhead.
    #[cfg(feature = "gpu")]
    const GPU_BRUTE_FORCE_THRESHOLD: usize = 100_000;
}
