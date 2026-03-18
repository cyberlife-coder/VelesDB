//! HNSW search methods: quality-based, reranking, and latency adaptation.
//!
//! Brute-force and GPU search methods are in `brute_force.rs`.

use super::HnswIndex;
use crate::distance::DistanceMetric;
use crate::index::hnsw::params::SearchQuality;
use crate::scored_result::ScoredResult;
use std::sync::atomic::Ordering;
use std::time::Instant;

impl HnswIndex {
    /// Sets a soft latency target (microseconds) for two-stage reranking adaptation.
    ///
    /// A value of `0` disables latency-aware adaptation.
    pub fn set_rerank_latency_target_us(&self, target_us: u64) {
        self.rerank_latency_target_us
            .store(target_us, Ordering::Relaxed);
    }

    /// Returns the configured soft latency target for reranking (microseconds).
    #[must_use]
    pub fn rerank_latency_target_us(&self) -> u64 {
        self.rerank_latency_target_us.load(Ordering::Relaxed)
    }

    /// Returns the exponential moving average of reranking latency (microseconds).
    #[must_use]
    pub fn rerank_latency_ema_us(&self) -> u64 {
        self.rerank_latency_ema_us.load(Ordering::Relaxed)
    }

    /// Validates that the query/vector dimension matches the index dimension.
    ///
    /// RF-2.7: Helper to eliminate 7x duplicated validation pattern.
    ///
    /// # Panics
    ///
    /// Panics if the dimension doesn't match.
    #[inline]
    pub(crate) fn validate_dimension(&self, data: &[f32], data_type: &str) {
        assert_eq!(
            data.len(),
            self.dimension,
            "{data_type} dimension mismatch: expected {}, got {}",
            self.dimension,
            data.len()
        );
    }

    /// Computes exact SIMD distance between query and vector based on metric.
    ///
    /// This helper eliminates code duplication across search methods.
    #[inline]
    pub(crate) fn compute_distance(&self, query: &[f32], vector: &[f32]) -> f32 {
        match self.metric {
            DistanceMetric::Cosine => crate::simd_native::cosine_similarity_native(query, vector),
            DistanceMetric::Euclidean => crate::simd_native::euclidean_native(query, vector),
            DistanceMetric::DotProduct => crate::simd_native::dot_product_native(query, vector),
            DistanceMetric::Hamming => crate::simd_native::hamming_distance_native(query, vector),
            DistanceMetric::Jaccard => crate::simd_native::jaccard_similarity_native(query, vector),
        }
    }

    /// Performs HNSW-only search (no reranking).
    ///
    /// `pub(crate)` to allow reuse in `batch.rs` for `search_batch_parallel`.
    pub(crate) fn search_hnsw_only(
        &self,
        query: &[f32],
        k: usize,
        ef_search: usize,
    ) -> Vec<ScoredResult> {
        let inner = self.inner.read();
        let neighbours = inner.search(query, k, ef_search);

        let mut results: Vec<ScoredResult> = Vec::with_capacity(neighbours.len());
        for n in &neighbours {
            if let Some(id) = self.mappings.get_id(n.d_id) {
                let score = inner.transform_score(n.distance);
                results.push(ScoredResult::new(id, score));
            }
        }
        results
    }

    /// Determines whether two-stage reranking should be used.
    ///
    /// Returns `Some(rerank_k)` if reranking is beneficial, `None` otherwise.
    fn should_two_stage_rerank(
        &self,
        quality: SearchQuality,
        k: usize,
        ef_search: usize,
    ) -> Option<usize> {
        // Skip reranking for Fast quality or if vector storage is disabled
        if matches!(quality, SearchQuality::Fast) || !self.enable_vector_storage {
            return None;
        }

        // Two-stage reranking: use a larger candidate pool for initial HNSW search,
        // then rerank with exact SIMD distances for better precision.
        let min_rerank_k = match quality {
            SearchQuality::Balanced => k * 2,
            SearchQuality::Accurate | SearchQuality::Custom(_) => k * 4,
            SearchQuality::Fast | SearchQuality::Perfect => return None,
        };

        let rerank_k = min_rerank_k.max(ef_search / 2);

        // Only rerank if we have enough vectors and rerank_k > k
        if rerank_k > k && self.len() > k * 2 {
            // Latency-aware adaptation
            let adapted = self.adapt_rerank_k_to_latency(rerank_k, k);
            Some(adapted)
        } else {
            None
        }
    }

    /// Adapts `rerank_k` based on observed latency vs target.
    fn adapt_rerank_k_to_latency(&self, rerank_k: usize, k: usize) -> usize {
        let target = self.rerank_latency_target_us.load(Ordering::Relaxed);
        if target == 0 {
            return rerank_k; // Adaptation disabled
        }

        let ema = self.rerank_latency_ema_us.load(Ordering::Relaxed);
        if ema == 0 {
            return rerank_k; // No data yet
        }

        // If we're over budget, reduce rerank_k proportionally
        if ema > target {
            let scaled = (rerank_k as u128).saturating_mul(u128::from(target)) / u128::from(ema);
            let adapted = usize::try_from(scaled).unwrap_or(usize::MAX);
            adapted.max(k) // Never go below k
        } else {
            rerank_k
        }
    }

    /// Updates the exponential moving average of reranking latency.
    fn update_rerank_latency_ema(&self, sample_us: u64) {
        let current = self.rerank_latency_ema_us.load(Ordering::Relaxed);
        if current == 0 {
            self.rerank_latency_ema_us
                .store(sample_us, Ordering::Relaxed);
        } else {
            // EMA with alpha=0.3 for responsiveness
            // Compute in u128 to avoid overflow in weighted sum.
            let weighted_sum = u128::from(current) * 7 + u128::from(sample_us) * 3;
            let new_ema = u64::try_from(weighted_sum / 10).unwrap_or(u64::MAX);
            self.rerank_latency_ema_us.store(new_ema, Ordering::Relaxed);
        }
    }

    /// Searches with adaptive quality using two-stage reranking.
    ///
    /// Uses the specified `SearchQuality` to determine ef_search and
    /// whether to apply two-stage SIMD reranking for improved precision.
    ///
    /// # Panics
    ///
    /// Panics if the query dimension doesn't match the index dimension.
    #[must_use]
    pub fn search_with_quality(
        &self,
        query: &[f32],
        k: usize,
        quality: SearchQuality,
    ) -> Vec<ScoredResult> {
        self.validate_dimension(query, "Query");

        // Perfect mode uses brute-force SIMD for guaranteed 100% recall
        if matches!(quality, SearchQuality::Perfect) {
            return self.search_brute_force(query, k);
        }

        // For very small collections (≤100 vectors), use brute-force to guarantee 100% recall
        // HNSW graph may not be fully connected with so few nodes, causing missed results
        // Only use brute-force if vector storage is enabled (not in fast-insert mode)
        if self.len() <= 100 && self.enable_vector_storage && !self.vectors.is_empty() {
            return self.search_brute_force(query, k);
        }

        let ef_search = quality.ef_search(k);

        // Two-stage mode: larger candidate pool + exact SIMD reranking.
        if let Some(rerank_k) = self.should_two_stage_rerank(quality, k, ef_search) {
            return self.search_with_rerank_with_ef(query, k, rerank_k, ef_search);
        }

        self.search_hnsw_only(query, k, ef_search)
    }

    /// Searches with SIMD-based re-ranking for improved precision.
    ///
    /// This method first retrieves `rerank_k` candidates using the HNSW index,
    /// then re-ranks them using our SIMD-optimized distance functions for
    /// exact distance computation, returning the top `k` results.
    ///
    /// # Panics
    ///
    /// Panics if the query dimension doesn't match the index dimension.
    #[must_use]
    pub fn search_with_rerank(
        &self,
        query: &[f32],
        k: usize,
        rerank_k: usize,
    ) -> Vec<ScoredResult> {
        let ef_search = SearchQuality::Accurate.ef_search(rerank_k);
        let adaptive_rerank_k = self
            .should_two_stage_rerank(SearchQuality::Accurate, k, ef_search)
            .unwrap_or(rerank_k.min(self.len().max(k)));
        self.search_with_rerank_with_ef(query, k, adaptive_rerank_k, ef_search)
    }

    fn search_with_rerank_with_ef(
        &self,
        query: &[f32],
        k: usize,
        rerank_k: usize,
        ef_search: usize,
    ) -> Vec<ScoredResult> {
        self.validate_dimension(query, "Query");

        // 1. Get candidates from HNSW (fast approximate search)
        let candidates = self.search_hnsw_only(query, rerank_k, ef_search);

        self.rerank_sort_and_truncate(query, &candidates, k)
    }

    /// Searches with SIMD-based re-ranking using a custom quality for initial search.
    ///
    /// # Panics
    ///
    /// Panics if the query dimension doesn't match the index dimension.
    #[must_use]
    pub fn search_with_rerank_quality(
        &self,
        query: &[f32],
        k: usize,
        rerank_k: usize,
        initial_quality: SearchQuality,
    ) -> Vec<ScoredResult> {
        self.validate_dimension(query, "Query");

        // 1. Get candidates from HNSW with specified quality
        // Avoid recursion if initial_quality is Perfect
        let actual_quality = if matches!(initial_quality, SearchQuality::Perfect) {
            SearchQuality::Accurate
        } else {
            initial_quality
        };
        let candidates = self.search_with_quality(query, rerank_k, actual_quality);

        self.rerank_sort_and_truncate(query, &candidates, k)
    }

    /// Reranks candidates with SIMD, sorts, truncates, and updates latency EMA.
    ///
    /// RF-2: Shared rerank pipeline used by both `search_with_rerank_with_ef`
    /// and `search_with_rerank_quality` to eliminate 4 duplicated lines.
    fn rerank_sort_and_truncate(
        &self,
        query: &[f32],
        candidates: &[ScoredResult],
        k: usize,
    ) -> Vec<ScoredResult> {
        if candidates.is_empty() {
            return Vec::new();
        }

        let rerank_start = Instant::now();

        let mut reranked = self.rerank_candidates(query, candidates);

        self.metric.sort_scored_results(&mut reranked);
        reranked.truncate(k);

        let elapsed_micros = rerank_start.elapsed().as_micros();
        let elapsed = u64::try_from(elapsed_micros).unwrap_or(u64::MAX);
        self.update_rerank_latency_ema(elapsed);
        reranked
    }

    /// Re-ranks candidates using SIMD-optimized exact distance computation.
    ///
    /// F-05: Zero-copy reranking — reads vector slices directly from
    /// `ContiguousVectors` (64-byte aligned, cache-friendly) instead of
    /// cloning via `ShardedVectors::get()`. Eliminates ~600KB of allocations
    /// per search for k=100 × dim=1536.
    fn rerank_candidates(&self, query: &[f32], candidates: &[ScoredResult]) -> Vec<ScoredResult> {
        let inner = self.inner.read();

        inner.with_contiguous_vectors(|vectors| {
            // Resolve external IDs → internal indices (no vector cloning)
            let candidate_indices: Vec<(u64, usize)> = candidates
                .iter()
                .filter_map(|sr| {
                    let idx = self.mappings.get_idx(sr.id)?;
                    Some((sr.id, idx))
                })
                .collect();

            let prefetch_distance = crate::simd_native::calculate_prefetch_distance(self.dimension);
            let mut reranked: Vec<ScoredResult> = Vec::with_capacity(candidate_indices.len());

            for (i, &(id, idx)) in candidate_indices.iter().enumerate() {
                // Prefetch upcoming vectors from contiguous storage
                if i + prefetch_distance < candidate_indices.len() {
                    vectors.prefetch(candidate_indices[i + prefetch_distance].1);
                }

                // Zero-copy: get &[f32] slice directly from ContiguousVectors
                if let Some(vec) = vectors.get(idx) {
                    let exact_dist = self.compute_distance(query, vec);
                    reranked.push(ScoredResult::new(id, exact_dist));
                }
            }

            reranked
        })
    }

    /// Sets the index to searching mode after bulk insertions.
    ///
    /// This is required by `hnsw_rs` after parallel insertions to ensure
    /// correct search results. Call this after finishing all insertions
    /// and before performing searches.
    pub fn set_searching_mode(&self) {
        // RF-1: Using HnswInner method
        let mut inner = self.inner.write();
        inner.set_searching_mode(true);
    }
}
