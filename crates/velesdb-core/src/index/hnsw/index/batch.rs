//! Batch operations for HnswIndex.

use super::HnswIndex;
use crate::index::hnsw::params::SearchQuality;
use crate::index::hnsw::sharded_mappings::SlotsPinned;
use crate::scored_result::ScoredResult;
use crate::validation::validate_dimension_match;
use rayon::prelude::*;

impl HnswIndex {
    /// Inserts multiple vectors in parallel using rayon.
    ///
    /// This method is optimized for bulk insertions and can significantly
    /// reduce indexing time on multi-core systems.
    ///
    /// # Ordering
    ///
    /// Every dimension is checked before the graph sees any vector. The graph
    /// then places the batch and returns each vector's slot in input order,
    /// and only then is each id mapped to its slot (#2246) — see
    /// `docs/SOUNDNESS.md` "HNSW Slot Allocation". An id repeated within the
    /// batch ends on its last occurrence. Because `parallel_insert` uses
    /// rayon, the HNSW graph construction order is non-deterministic across
    /// runs (see v1.7.2 CHANGELOG note).
    ///
    /// # Arguments
    ///
    /// * `vectors` - Iterator of (id, vector) pairs to insert
    ///
    /// # Returns
    ///
    /// Number of vectors inserted or updated (upsert semantics). A batch
    /// holding a vector of the wrong dimension, or one the graph refuses,
    /// maps nothing and returns 0, with the cause logged; nodes the graph had
    /// already placed stay unmapped, as tombstones.
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
        let items: Vec<(u64, &'a [f32])> = vectors.into_iter().collect();
        if let Some(e) = items
            .iter()
            .find_map(|(_, vector)| validate_dimension_match(self.dimension, vector.len()).err())
        {
            tracing::error!("insert_batch_parallel: dimension validation failed: {e}");
            return 0;
        }
        if items.is_empty() {
            return 0;
        }

        let vectors: Vec<&[f32]> = items.iter().map(|(_, vector)| *vector).collect();
        // Held until every id is mapped: `reorder_for_locality` and `vacuum`
        // renumber slots under the write lock, so each slot placed here is still
        // its vector's when the mapping names it.
        let inner = self.inner.read();
        let placed = inner.parallel_insert(&vectors).map(|slots| {
            for ((id, _), slot) in items.iter().zip(slots) {
                self.mappings
                    .assign(*id, slot, SlotsPinned::by_read(&inner));
            }
        });
        drop(inner);
        match placed {
            Ok(()) => items.len(),
            Err(e) => {
                tracing::error!("insert_batch_parallel: parallel_insert failed: {e}");
                0
            }
        }
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
    /// Returns [`crate::error::Error::DimensionMismatch`] if any query dimension does not
    /// match the index dimension.
    pub fn search_batch_parallel(
        &self,
        queries: &[&[f32]],
        k: usize,
        quality: SearchQuality,
    ) -> crate::error::Result<Vec<Vec<ScoredResult>>> {
        self.validate_batch_dimensions(queries)?;

        // Perfect, Adaptive, AutoTune, or very small collections: delegate to
        // search_with_quality per-query to match single-query behavior.
        // - Perfect: uses brute-force for 100% recall
        // - Adaptive: uses spread-based two-phase escalation (not batch-compatible)
        // - AutoTune: computes auto-ef range per dataset/dim/k (issue #699 follow-up)
        // - Small (<=100): uses brute-force for fully-connected graph safety
        //
        // Without AutoTune in this list, batch + AutoTune would fall through to the
        // standard ef_search_for_scale HNSW path below — same fixed quality for every
        // query — which silently disables the adaptive ef-range mechanism that the
        // single-query path applies via try_search_special_quality.
        if matches!(
            quality,
            SearchQuality::Perfect | SearchQuality::Adaptive { .. } | SearchQuality::AutoTune
        ) || (self.len() <= 100 && self.enable_vector_storage && self.graph_vector_count() > 0)
        {
            let results: crate::error::Result<Vec<Vec<ScoredResult>>> = queries
                .par_iter()
                .map(|query| self.search_with_quality(query, k, quality))
                .collect();
            return results;
        }

        // Aligned with single-query path (search.rs:339): use ef_search_for_scale
        // so batch and single-query produce the same ef value for the same quality
        // profile when dataset_size > 10K. Without this, batch queries on large
        // datasets would silently use a lower ef than equivalent single queries,
        // breaking the implicit contract that single and batch with the same
        // quality return the same results. See issue #694.
        let ef_search = quality.ef_search_for_scale(k, self.len());

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
    /// Returns [`crate::error::Error::DimensionMismatch`] on the first query whose dimension
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

        let total_us: u64 = timed_results.iter().map(|(_, e)| e).sum();
        if let Some(mean_us) = total_us.checked_div(timed_results.len() as u64) {
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
    /// Returns [`crate::error::Error::DimensionMismatch`] if the query dimension does not
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

    /// Rayon-based brute-force search over the graph's `ContiguousVectors`.
    ///
    /// Extracted from `brute_force_search_parallel` so the GPU gate in that
    /// method stays compact. Also used by `search_brute_force` as the default
    /// compute path (RF-DEDUP: single parallel implementation).
    ///
    /// Snapshots the contiguous slab under a brief read lock (one memcpy),
    /// then releases it BEFORE the rayon scan: parallel inserts acquire
    /// `vectors.write()` from inside rayon tasks, so running parallel work
    /// while holding the vectors lock could park every worker on that write
    /// and deadlock the pool (same discipline as the GPU snapshot paths).
    ///
    /// Internal indices `0..count` are filtered through `mappings.get_id`,
    /// so tombstoned slots (deleted/upserted vectors that remain in the
    /// graph store) are never returned.
    pub(super) fn brute_force_search_rayon(&self, query: &[f32], k: usize) -> Vec<ScoredResult> {
        // Preserve fast-insert semantics: exact-distance features are gated
        // on `enable_vector_storage` even though the graph stores vectors.
        if !self.enable_vector_storage {
            return Vec::new();
        }

        let (flat, dimension) = {
            let inner = self.inner.read();
            inner.with_contiguous_vectors(|vectors| {
                (vectors.as_flat_slice().to_vec(), vectors.dimension())
            })
        };
        if flat.is_empty() || dimension == 0 {
            return Vec::new();
        }

        let mut results: Vec<ScoredResult> = flat
            .par_chunks(dimension)
            .enumerate()
            .filter_map(|(idx, vec)| {
                let id = self.mappings.get_id(idx)?;
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
