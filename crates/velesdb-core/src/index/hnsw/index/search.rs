//! HNSW search methods: quality-based, reranking, and latency adaptation.
//!
//! Brute-force and GPU search methods are in `brute_force.rs`.

use super::HnswIndex;
use crate::distance::DistanceMetric;
use crate::index::hnsw::params::SearchQuality;
use crate::scored_result::ScoredResult;
use crate::validation::validate_dimension_match;
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
    /// # Errors
    ///
    /// Returns [`Error::DimensionMismatch`] if `data.len() != self.dimension`.
    #[inline]
    pub(crate) fn validate_dimension(&self, data: &[f32]) -> crate::error::Result<()> {
        validate_dimension_match(self.dimension, data.len())
    }

    /// Computes exact SIMD distance between query and vector based on metric.
    ///
    /// This helper eliminates code duplication across search methods.
    ///
    /// # Invariant
    ///
    /// This method intentionally uses metric-specific `simd_native` functions
    /// (e.g. `euclidean_native` which includes sqrt) rather than the HNSW inner
    /// engine's `compute_distance()` (which returns squared L2 for Euclidean
    /// via `CachedSimdDistance`). This ensures reranking scores in
    /// `rerank_candidates_simd` are already in the user-visible metric space
    /// and do not require a subsequent `transform_score()` call. Changing this
    /// to use the inner engine's `compute_distance()` would produce squared L2
    /// scores that break reranking sort order against `transform_score`-applied
    /// HNSW results.
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
        for &(node_id, raw_dist) in &neighbours {
            if let Some(id) = self.mappings.get_id(node_id) {
                let score = inner.transform_score(raw_dist);
                results.push(ScoredResult::new(id, score));
            }
        }
        results
    }

    /// Performs HNSW search with a pre-filter bitmap.
    ///
    /// Over-fetches candidates from the HNSW graph (using `ef_search` as the
    /// candidate pool size), then retains only results whose external ID is
    /// present in `allowed_ids`. This avoids the expensive payload-level
    /// post-filter for points that cannot match.
    ///
    /// Graph connectivity is preserved: the HNSW traversal explores all
    /// nodes for navigation, but only bitmap-matching nodes appear in the
    /// output. This maintains recall quality.
    pub(crate) fn search_hnsw_only_filtered(
        &self,
        query: &[f32],
        candidates_k: usize,
        ef_search: usize,
        allowed_ids: &roaring::RoaringBitmap,
    ) -> Vec<ScoredResult> {
        let inner = self.inner.read();
        let neighbours = inner.search(query, candidates_k, ef_search);

        let mut results: Vec<ScoredResult> = Vec::with_capacity(neighbours.len());
        for &(node_id, raw_dist) in &neighbours {
            if let Some(id) = self.mappings.get_id(node_id) {
                // Bitmap uses u32 keys. IDs in the bitmap must match.
                // IDs > u32::MAX cannot be represented in the bitmap, so they
                // are passed through unconditionally to the post-filter.
                let in_bitmap = u32::try_from(id).is_ok_and(|id32| allowed_ids.contains(id32));
                let exceeds_bitmap_range = u32::try_from(id).is_err();
                if in_bitmap || exceeds_bitmap_range {
                    let score = inner.transform_score(raw_dist);
                    results.push(ScoredResult::new(id, score));
                }
            }
        }
        results
    }

    /// Determines whether two-stage reranking should be used.
    ///
    /// Returns `Some(rerank_k)` if reranking is beneficial, `None` otherwise.
    ///
    /// # Fast mode behavior
    ///
    /// `SearchQuality::Fast` enables reranking with `min_rerank_k = k` (no
    /// oversampling). This performs rank correction — re-scoring the top-k
    /// HNSW candidates with exact SIMD distances — without the latency
    /// penalty of a larger candidate pool. The result is better rank ordering
    /// at negligible cost, since the HNSW search itself already fetched these
    /// candidates.
    ///
    /// Higher quality modes (`Balanced`, `Accurate`) use progressively larger
    /// oversampling factors (3x, 4x) to pull in candidates that HNSW's greedy
    /// search might miss, at the cost of additional distance computations.
    pub(super) fn should_two_stage_rerank(
        &self,
        quality: SearchQuality,
        k: usize,
        ef_search: usize,
    ) -> Option<usize> {
        // Skip reranking for Adaptive or AutoTune quality (these handle
        // their own exploration strategy) or if vector storage is disabled.
        if matches!(
            quality,
            SearchQuality::Adaptive { .. } | SearchQuality::AutoTune
        ) || !self.enable_vector_storage
        {
            return None;
        }

        // Two-stage reranking: use a larger candidate pool for initial HNSW search,
        // then rerank with exact SIMD distances for better precision.
        // Fast mode uses k-only reranking (no oversampling, just re-score for
        // rank correction). Balanced uses 3x for 0.995+ recall.
        let min_rerank_k = match quality {
            SearchQuality::Fast => k,
            SearchQuality::Balanced => k * 3,
            SearchQuality::Accurate | SearchQuality::Custom(_) => k * 4,
            SearchQuality::Perfect | SearchQuality::Adaptive { .. } | SearchQuality::AutoTune => {
                return None;
            }
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
    ///
    /// The load-compute-store sequence is intentionally non-atomic (no CAS loop).
    /// Two concurrent searches may each read the same `current` value and both
    /// store their own blended result, causing one sample to be silently dropped.
    /// This is acceptable because the EMA is a best-effort heuristic for latency
    /// adaptation -- occasional lost samples do not affect search correctness and
    /// the overhead of a CAS retry loop is not justified for an advisory metric.
    pub(super) fn update_rerank_latency_ema(&self, sample_us: u64) {
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
    /// # Errors
    ///
    /// Returns [`Error::DimensionMismatch`] if the query dimension does not
    /// match the index dimension.
    pub fn search_with_quality(
        &self,
        query: &[f32],
        k: usize,
        quality: SearchQuality,
    ) -> crate::error::Result<Vec<ScoredResult>> {
        self.validate_dimension(query)?;

        // Perfect mode uses brute-force SIMD for guaranteed 100% recall
        if matches!(quality, SearchQuality::Perfect) {
            return self.search_brute_force(query, k);
        }

        // For very small collections (<=100 vectors), use brute-force to guarantee 100% recall
        // HNSW graph may not be fully connected with so few nodes, causing missed results
        // Only use brute-force if vector storage is enabled (not in fast-insert mode)
        if self.len() <= 100 && self.enable_vector_storage && !self.vectors.is_empty() {
            return self.search_brute_force(query, k);
        }

        // Adaptive two-phase: start with min_ef, escalate if query is hard
        if let SearchQuality::Adaptive { min_ef, max_ef } = quality {
            return Ok(self.search_adaptive(query, k, min_ef.max(k), max_ef));
        }

        // AutoTune: compute ef range from collection statistics, then delegate
        // to the same adaptive two-phase algorithm.
        if matches!(quality, SearchQuality::AutoTune) {
            let (min_ef, max_ef) =
                crate::index::hnsw::auto_ef::auto_ef_range(self.len(), self.dimension, k);
            return Ok(self.search_adaptive(query, k, min_ef, max_ef));
        }

        let ef_search = quality.ef_search_for_scale(k, self.len());

        // Two-stage mode: larger candidate pool + exact SIMD reranking.
        if let Some(rerank_k) = self.should_two_stage_rerank(quality, k, ef_search) {
            return self.search_with_rerank_with_ef(query, k, rerank_k, ef_search);
        }

        Ok(self.search_hnsw_only(query, k, ef_search))
    }

    /// Two-phase adaptive search that starts with a low ef and escalates if needed.
    ///
    /// Phase 1: search with `min_ef`. If the result spread (max_dist / min_dist)
    /// indicates a hard query (scattered results), re-search with doubled ef.
    /// This saves 2-4x latency on easy queries while maintaining recall on hard ones.
    fn search_adaptive(
        &self,
        query: &[f32],
        k: usize,
        min_ef: usize,
        max_ef: usize,
    ) -> Vec<ScoredResult> {
        // Phase 1: fast search with min_ef
        let results = self.search_hnsw_only(query, k, min_ef);
        if results.len() < 2 {
            return results;
        }

        // Check result spread to determine if this is a hard query.
        // Use first/last scores -- ordering differs by metric (similarity=desc,
        // distance=asc) so we take the absolute spread.
        let score_a = results.first().map_or(0.0, |r| r.score);
        let score_b = results.last().map_or(0.0, |r| r.score);
        let diff = (score_a - score_b).abs();

        // Relative spread: normalize by the smaller absolute score to handle
        // both similarity metrics (high scores) and distance metrics (low scores).
        let baseline = score_a.abs().min(score_b.abs());
        let spread = if baseline > f32::EPSILON {
            diff / baseline
        } else {
            diff
        };

        // Threshold 2.0: empirically tuned. Easy queries have spread < 1.0,
        // hard queries typically > 3.0.
        if spread < 2.0 {
            return results;
        }

        // Phase 2: re-search with doubled ef (capped at max_ef)
        let escalated_ef = (min_ef * 2).min(max_ef);
        if escalated_ef <= min_ef {
            return results;
        }

        self.search_hnsw_only(query, k, escalated_ef)
    }

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

    fn search_with_rerank_with_ef(
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

    /// Sets the index to searching mode after bulk insertions.
    ///
    /// This is required by `hnsw_rs` after parallel insertions to ensure
    /// correct search results. Call this after finishing all insertions
    /// and before performing searches.
    pub fn set_searching_mode(&self) {
        let mut inner = self.inner.write();
        inner.set_searching_mode(true);
    }
}
