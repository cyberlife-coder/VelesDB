//! HNSW two-stage reranking methods extracted from `search.rs`.
//!
//! Contains SIMD and GPU reranking, candidate resolution, and latency-aware
//! rerank adaptation for the `HnswIndex`.

use super::HnswIndex;
use crate::index::hnsw::params::SearchQuality;
use crate::scored_result::ScoredResult;
use std::time::Instant;

impl HnswIndex {
    /// Searches with SIMD-based re-ranking for improved precision.
    ///
    /// This method first retrieves `rerank_k` candidates using the HNSW index,
    /// then re-ranks them using our SIMD-optimized distance functions for
    /// exact distance computation, returning the top `k` results.
    ///
    /// # Errors
    ///
    /// Returns [`Error::DimensionMismatch`] if the query dimension does not
    /// match the index dimension.
    pub fn search_with_rerank(
        &self,
        query: &[f32],
        k: usize,
        rerank_k: usize,
    ) -> crate::error::Result<Vec<ScoredResult>> {
        let ef_search = SearchQuality::Accurate.ef_search(rerank_k);
        let adaptive_rerank_k = self
            .should_two_stage_rerank(SearchQuality::Accurate, k, ef_search)
            .unwrap_or(rerank_k.min(self.len().max(k)));
        self.search_with_rerank_with_ef(query, k, adaptive_rerank_k, ef_search)
    }

    pub(crate) fn search_with_rerank_with_ef(
        &self,
        query: &[f32],
        k: usize,
        rerank_k: usize,
        ef_search: usize,
    ) -> crate::error::Result<Vec<ScoredResult>> {
        self.validate_dimension(query)?;
        let candidates = self.search_hnsw_only(query, rerank_k, ef_search);

        Ok(self.rerank_sort_and_truncate(query, &candidates, k))
    }

    /// Searches with SIMD-based re-ranking using a custom quality for initial search.
    ///
    /// # Errors
    ///
    /// Returns [`Error::DimensionMismatch`] if the query dimension does not
    /// match the index dimension.
    pub fn search_with_rerank_quality(
        &self,
        query: &[f32],
        k: usize,
        rerank_k: usize,
        initial_quality: SearchQuality,
    ) -> crate::error::Result<Vec<ScoredResult>> {
        self.validate_dimension(query)?;

        // Avoid recursion if initial_quality is Perfect
        let actual_quality = if matches!(initial_quality, SearchQuality::Perfect) {
            SearchQuality::Accurate
        } else {
            initial_quality
        };
        let candidates = self.search_with_quality(query, rerank_k, actual_quality)?;

        Ok(self.rerank_sort_and_truncate(query, &candidates, k))
    }

    /// Reranks candidates with SIMD, sorts, truncates, and updates latency EMA.
    ///
    /// The batch path in `batch.rs` uses `rerank_sort_and_truncate_timed`
    /// directly for aggregated EMA updates.
    pub(super) fn rerank_sort_and_truncate(
        &self,
        query: &[f32],
        candidates: &[ScoredResult],
        k: usize,
    ) -> Vec<ScoredResult> {
        let (results, elapsed) = self.rerank_sort_and_truncate_timed(query, candidates, k);
        if elapsed > 0 {
            self.update_rerank_latency_ema(elapsed);
        }
        results
    }

    /// Reranks, sorts, and truncates without updating the EMA.
    ///
    /// Returns `(results, elapsed_us)` so the caller can aggregate latencies
    /// from a parallel batch and update the EMA once (avoiding lost samples).
    pub(super) fn rerank_sort_and_truncate_timed(
        &self,
        query: &[f32],
        candidates: &[ScoredResult],
        k: usize,
    ) -> (Vec<ScoredResult>, u64) {
        if candidates.is_empty() {
            return (Vec::new(), 0);
        }

        let rerank_start = Instant::now();

        let mut reranked = self.rerank_candidates(query, candidates);

        self.metric.sort_scored_results(&mut reranked);
        reranked.truncate(k);

        let elapsed_micros = rerank_start.elapsed().as_micros();
        let elapsed = u64::try_from(elapsed_micros).unwrap_or(u64::MAX);
        (reranked, elapsed)
    }

    /// Re-ranks candidates using the best available compute path.
    ///
    /// Tries GPU dispatch first when the workload exceeds the GPU threshold
    /// (rerank_k * dimension > 262,144 floats, ~1 MB) and a GPU is available.
    /// Falls back to SIMD for small workloads, unsupported metrics, or GPU errors.
    fn rerank_candidates(&self, query: &[f32], candidates: &[ScoredResult]) -> Vec<ScoredResult> {
        #[cfg(feature = "gpu")]
        {
            use crate::gpu::GpuAccelerator;
            if GpuAccelerator::should_rerank_gpu(candidates.len(), self.dimension) {
                if let Some(results) = self.rerank_candidates_gpu(query, candidates) {
                    return results;
                }
            }
        }
        self.rerank_candidates_simd(query, candidates)
    }

    /// Resolves candidate external IDs to internal indices.
    ///
    /// Shared by both SIMD and GPU reranking paths to eliminate duplication.
    fn resolve_candidate_indices(&self, candidates: &[ScoredResult]) -> Vec<(u64, usize)> {
        candidates
            .iter()
            .filter_map(|sr| {
                let idx = self.mappings.get_idx(sr.id)?;
                Some((sr.id, idx))
            })
            .collect()
    }

    /// Clamps a GPU-computed score to the mathematical range of the metric.
    ///
    /// GPU shaders use f32 with different reduction trees than CPU SIMD, so
    /// floating-point rounding can push bounded metrics (Cosine, Jaccard)
    /// slightly outside their theoretical range. Clamping guarantees
    /// downstream assertions and comparisons are never violated.
    ///
    /// Only Cosine ([-1, 1]) and Jaccard ([0, 1]) are bounded.
    /// DotProduct, Euclidean, and Hamming are unbounded.
    #[cfg(feature = "gpu")]
    #[inline]
    pub(crate) fn clamp_score_for_metric(&self, score: f32) -> f32 {
        use crate::distance::DistanceMetric;
        match self.metric {
            DistanceMetric::Cosine => score.clamp(-1.0, 1.0),
            DistanceMetric::Jaccard => score.clamp(0.0, 1.0),
            // DotProduct, Euclidean, Hamming: unbounded
            _ => score,
        }
    }

    /// Re-ranks candidates using GPU batch distance computation.
    ///
    /// Snapshots candidate vectors under a brief read lock, then releases
    /// the lock before the GPU round-trip (buffer upload + compute + poll +
    /// readback = 5-50 ms). This prevents writer starvation during GPU dispatch.
    ///
    /// Returns `None` if GPU is unavailable, the metric has no GPU shader,
    /// or a GPU error occurs. The caller falls back to SIMD in that case.
    #[cfg(feature = "gpu")]
    pub(crate) fn rerank_candidates_gpu(
        &self,
        query: &[f32],
        candidates: &[ScoredResult],
    ) -> Option<Vec<ScoredResult>> {
        use crate::gpu::GpuAccelerator;

        let gpu = GpuAccelerator::global()?;

        // Snapshot vectors under a brief read lock, then release before GPU dispatch
        let (entries, flat_vectors) = {
            let inner = self.inner.read();
            inner.with_contiguous_vectors(|vectors| {
                let entries = self.resolve_candidate_indices(candidates);
                if entries.is_empty() {
                    return None;
                }
                let indices: Vec<usize> = entries.iter().map(|&(_, idx)| idx).collect();
                let flat = vectors.gather_flat(&indices);
                // Early validation: gather_flat may skip invalidated indices,
                // producing fewer elements. Detect before paying GPU round-trip.
                let expected_len = indices.len() * self.dimension;
                if flat.len() != expected_len {
                    return None;
                }
                Some((entries, flat))
            })
        }?;

        // Lock released -- GPU dispatch is lock-free
        let scores = gpu
            .batch_distance_for_metric(self.metric, &flat_vectors, query, self.dimension)?
            .ok()?;

        // Guard: GPU must return exactly one score per entry. If mismatched
        // (e.g., shader error or buffer desync), fall back to SIMD.
        if scores.len() != entries.len() {
            return None;
        }

        let reranked = entries
            .iter()
            .zip(scores.iter())
            .map(|(&(id, _), &score)| ScoredResult::new(id, self.clamp_score_for_metric(score)))
            .collect();

        Some(reranked)
    }

    /// Re-ranks candidates using SIMD-optimized exact distance computation.
    ///
    /// Reads vector slices directly from `ContiguousVectors` (64-byte aligned,
    /// cache-friendly) instead of cloning via `ShardedVectors::get()`.
    pub(crate) fn rerank_candidates_simd(
        &self,
        query: &[f32],
        candidates: &[ScoredResult],
    ) -> Vec<ScoredResult> {
        let inner = self.inner.read();

        inner.with_contiguous_vectors(|vectors| {
            let candidate_indices = self.resolve_candidate_indices(candidates);

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
}
