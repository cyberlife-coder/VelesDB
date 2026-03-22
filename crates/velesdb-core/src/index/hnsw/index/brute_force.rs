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
    /// Performs brute-force SIMD search for guaranteed 100% recall.
    ///
    /// This method computes exact distances to all vectors using SIMD-optimized
    /// functions and returns the top k results.
    ///
    /// # Performance
    ///
    /// O(n) where n = number of vectors. Best for small indices (<10k vectors)
    /// or when perfect recall is required.
    ///
    /// # Panics
    ///
    /// Panics if the query dimension doesn't match the index dimension.
    #[must_use]
    pub fn search_brute_force(&self, query: &[f32], k: usize) -> Vec<ScoredResult> {
        self.validate_dimension(query, "Query");

        // If vector storage is disabled, fall back to HNSW graph search.
        // RF-DEDUP: reuse search_hnsw_only instead of duplicating neighbour mapping.
        if !self.enable_vector_storage || self.vectors.is_empty() {
            let ef_search = SearchQuality::Accurate.ef_search(k);
            return self.search_hnsw_only(query, k, ef_search);
        }

        // Compute distances to all vectors
        let prefetch_distance = crate::simd_native::calculate_prefetch_distance(self.dimension);
        let vectors_snapshot: Vec<(usize, Vec<f32>)> = self.vectors.collect_for_parallel();

        let mut results: Vec<ScoredResult> = Vec::with_capacity(vectors_snapshot.len());

        for (i, (idx, v)) in vectors_snapshot.iter().enumerate() {
            // Prefetch upcoming vectors
            if i + prefetch_distance < vectors_snapshot.len() {
                crate::simd_native::prefetch_vector(&vectors_snapshot[i + prefetch_distance].1);
            }

            if let Some(id) = self.mappings.get_id(*idx) {
                let score = self.compute_distance(query, v);
                results.push(ScoredResult::new(id, score));
            }
        }

        // Sort by distance (metric-dependent ordering)
        self.metric.sort_scored_results(&mut results);

        results.truncate(k);
        results
    }

    /// Performs GPU-accelerated brute-force search if available.
    ///
    /// Uses `ContiguousVectors::gather_flat()` to produce a single contiguous
    /// buffer for GPU upload, avoiding per-vector heap allocations from the
    /// older `collect_for_parallel()` path.
    ///
    /// Returns `None` if GPU feature is not enabled or GPU is not available.
    ///
    /// # Panics
    ///
    /// Panics if the query dimension doesn't match the index dimension.
    #[must_use]
    pub fn search_brute_force_gpu(&self, query: &[f32], k: usize) -> Option<Vec<ScoredResult>> {
        self.validate_dimension(query, "Query");

        #[cfg(feature = "gpu")]
        {
            self.search_brute_force_gpu_inner(query, k)
        }

        #[cfg(not(feature = "gpu"))]
        {
            let _ = (query, k); // Suppress unused warnings
            None
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
    /// # Panics
    ///
    /// Panics if the query dimension doesn't match the index dimension.
    #[must_use]
    pub fn search_brute_force_buffered(&self, query: &[f32], k: usize) -> Vec<ScoredResult> {
        // Currently identical to search_brute_force - buffer reuse is internal optimization
        self.search_brute_force(query, k)
    }
}
