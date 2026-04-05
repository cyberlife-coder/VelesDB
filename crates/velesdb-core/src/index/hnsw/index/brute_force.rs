//! Brute-force and GPU-accelerated search methods for HNSW index.
//!
//! Extracted from `search.rs` for single-responsibility:
//! - `search_brute_force`: SIMD-optimized exact search for small indices
//! - `search_brute_force_gpu`: GPU-accelerated search via wgpu
//! - `search_brute_force_buffered`: Buffer-reuse variant

use super::HnswIndex;
use crate::index::hnsw::params::SearchQuality;
use crate::scored_result::ScoredResult;

impl HnswIndex {
    /// Brute-force scan restricted to vectors in the bitmap.
    ///
    /// Iterates over bitmap IDs, resolves each to an internal index via
    /// `mappings`, retrieves the vector from `ContiguousVectors`, computes
    /// the exact SIMD distance, and returns the top-k results sorted by
    /// the index metric.
    ///
    /// IDs exceeding `u32::MAX` are not representable in `RoaringBitmap`
    /// and are unconditionally included (consistent with the HNSW+bitmap
    /// path in `search_bitmap_filtered_inner`).
    ///
    /// # Errors
    ///
    /// Returns [`Error::DimensionMismatch`] if query dimension is wrong.
    pub fn full_scan_with_bitmap(
        &self,
        query: &[f32],
        k: usize,
        allowed_ids: &roaring::RoaringBitmap,
    ) -> crate::error::Result<Vec<ScoredResult>> {
        self.validate_dimension(query)?;

        if allowed_ids.is_empty() {
            return Ok(Vec::new());
        }

        let inner = self.inner.read();
        let results = inner.with_contiguous_vectors(|vectors| {
            let mut scored: Vec<ScoredResult> =
                Vec::with_capacity(usize::try_from(allowed_ids.len()).unwrap_or(k));

            // Scan bitmap IDs (u32 range).
            self.score_bitmap_ids(query, allowed_ids, vectors, &mut scored);

            // Include IDs exceeding u32::MAX that cannot be represented in
            // RoaringBitmap — consistent with HNSW+bitmap path (search.rs).
            self.score_overflow_ids(query, vectors, &mut scored);

            self.metric.sort_scored_results(&mut scored);
            scored.truncate(k);
            scored
        });

        Ok(results)
    }

    /// Scores vectors whose IDs are present in the bitmap (u32 range).
    fn score_bitmap_ids(
        &self,
        query: &[f32],
        allowed_ids: &roaring::RoaringBitmap,
        vectors: &crate::perf_optimizations::ContiguousVectors,
        scored: &mut Vec<ScoredResult>,
    ) {
        for id32 in allowed_ids {
            let id = u64::from(id32);
            if let Some(idx) = self.mappings.get_idx(id) {
                if let Some(vec) = vectors.get(idx) {
                    let dist = self.compute_distance(query, vec);
                    scored.push(ScoredResult::new(id, dist));
                }
            }
        }
    }

    /// Scores vectors whose IDs exceed `u32::MAX` (not representable in `RoaringBitmap`).
    ///
    /// These are unconditionally included, consistent with the HNSW+bitmap
    /// path in `search_bitmap_filtered_inner`.
    fn score_overflow_ids(
        &self,
        query: &[f32],
        vectors: &crate::perf_optimizations::ContiguousVectors,
        scored: &mut Vec<ScoredResult>,
    ) {
        // Fast path: if no vectors are stored, nothing to scan.
        if vectors.is_empty() {
            return;
        }
        for idx in 0..vectors.len() {
            let Some(id) = self.mappings.get_id(idx) else {
                continue;
            };
            if u32::try_from(id).is_ok() {
                continue;
            }
            if let Some(vec) = vectors.get(idx) {
                let dist = self.compute_distance(query, vec);
                scored.push(ScoredResult::new(id, dist));
            }
        }
    }

    /// Performs brute-force search for guaranteed 100% recall.
    ///
    /// Uses rayon-parallelized distance computation across all stored vectors.
    /// Falls back to HNSW graph search when vector storage is disabled.
    ///
    /// # Performance
    ///
    /// O(n / cores) where n = number of vectors. Best for small indices
    /// (<10k vectors) or when perfect recall is required.
    ///
    /// # Errors
    ///
    /// Returns [`Error::DimensionMismatch`] if the query dimension does not
    /// match the index dimension.
    pub fn search_brute_force(
        &self,
        query: &[f32],
        k: usize,
    ) -> crate::error::Result<Vec<ScoredResult>> {
        self.validate_dimension(query)?;

        // If vector storage is disabled, fall back to HNSW graph search.
        // RF-DEDUP: reuse search_hnsw_only instead of duplicating neighbour mapping.
        if !self.enable_vector_storage || self.vectors.is_empty() {
            let ef_search = SearchQuality::Accurate.ef_search(k);
            return Ok(self.search_hnsw_only(query, k, ef_search));
        }

        // RF-DEDUP: delegate to rayon-parallelized implementation
        Ok(self.brute_force_search_rayon(query, k))
    }

    /// Performs GPU-accelerated brute-force search if available.
    ///
    /// Uses `ContiguousVectors::gather_flat()` to produce a single contiguous
    /// buffer for GPU upload, avoiding per-vector heap allocations from the
    /// older `collect_for_parallel()` path.
    ///
    /// Returns `None` if GPU feature is not enabled or GPU is not available.
    ///
    /// # Errors
    ///
    /// Returns [`Error::DimensionMismatch`] if the query dimension does not
    /// match the index dimension.
    pub fn search_brute_force_gpu(
        &self,
        query: &[f32],
        k: usize,
    ) -> crate::error::Result<Option<Vec<ScoredResult>>> {
        self.validate_dimension(query)?;

        #[cfg(feature = "gpu")]
        {
            Ok(self.search_brute_force_gpu_inner(query, k))
        }

        #[cfg(not(feature = "gpu"))]
        {
            let _ = (query, k); // Suppress unused warnings
            Ok(None)
        }
    }

    /// GPU brute-force inner implementation using `ContiguousVectors`.
    ///
    /// Snapshots all valid vectors under a brief read lock, then releases
    /// the lock before the GPU round-trip (buffer upload + compute + poll +
    /// readback = 5-50 ms). This prevents writer starvation during GPU dispatch.
    ///
    /// Separated from `search_brute_force_gpu` to keep the `#[cfg]` blocks
    /// minimal and the logic testable.
    ///
    /// RF-DEDUP: `pub(crate)` so `batch.rs` can reuse this for
    /// `brute_force_search_gpu_dispatch` instead of duplicating the logic.
    #[cfg(feature = "gpu")]
    pub(crate) fn search_brute_force_gpu_inner(
        &self,
        query: &[f32],
        k: usize,
    ) -> Option<Vec<ScoredResult>> {
        use crate::gpu::GpuAccelerator;

        let gpu = GpuAccelerator::global()?;

        // Snapshot vectors under a brief read lock, then release before GPU dispatch
        let (id_map, flat_vectors) = {
            let inner = self.inner.read();
            inner.with_contiguous_vectors(|vectors| {
                let (indices, id_map) = self.build_brute_force_id_map(vectors.len());
                if id_map.is_empty() {
                    return None;
                }
                let flat = vectors.gather_flat(&indices);
                // Concurrent deletion can make gather_flat skip invalidated indices,
                // producing fewer elements than expected. Detect the desync and fall
                // back to CPU search (caller treats None as "GPU unavailable").
                let expected_len = indices.len() * vectors.dimension();
                if flat.len() != expected_len {
                    return None;
                }
                Some((id_map, flat))
            })
        }?;

        // Lock released -- GPU dispatch is lock-free
        let scores = gpu
            .batch_distance_for_metric(self.metric, &flat_vectors, query, self.dimension)?
            .ok()?;

        // Guard: GPU must return exactly one score per vector. If mismatched
        // (e.g., shader error or buffer desync), fall back to CPU search.
        if scores.len() != id_map.len() {
            return None;
        }

        let mut results: Vec<ScoredResult> = id_map
            .into_iter()
            .zip(scores)
            .map(|(id, score)| ScoredResult::new(id, self.clamp_score_for_metric(score)))
            .collect();

        self.metric.sort_scored_results(&mut results);
        results.truncate(k);
        Some(results)
    }

    /// Builds parallel `indices` and `id_map` vectors for GPU brute-force.
    ///
    /// Iterates all internal indices `0..count`, keeping only those that have
    /// a valid external ID mapping (i.e., not deleted).
    #[cfg(feature = "gpu")]
    fn build_brute_force_id_map(&self, count: usize) -> (Vec<usize>, Vec<u64>) {
        let mut indices = Vec::with_capacity(count);
        let mut id_map = Vec::with_capacity(count);
        for idx in 0..count {
            if let Some(id) = self.mappings.get_id(idx) {
                indices.push(idx);
                id_map.push(id);
            }
        }
        (indices, id_map)
    }

    /// Performs brute-force SIMD search with buffer reuse optimization.
    ///
    /// This is functionally identical to `search_brute_force` but may reuse
    /// internal buffers for better performance in repeated calls.
    ///
    /// # Performance
    ///
    /// O(n) where n = number of vectors. Best for small indices (<10k vectors)
    /// or when perfect recall is required.
    ///
    /// # Errors
    ///
    /// Returns [`Error::DimensionMismatch`] if the query dimension does not
    /// match the index dimension.
    pub fn search_brute_force_buffered(
        &self,
        query: &[f32],
        k: usize,
    ) -> crate::error::Result<Vec<ScoredResult>> {
        // Currently identical to search_brute_force - buffer reuse is internal optimization
        self.search_brute_force(query, k)
    }
}
