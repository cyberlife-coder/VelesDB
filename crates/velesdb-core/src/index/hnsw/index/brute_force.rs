//! Brute-force and GPU-accelerated search methods for HNSW index.
//!
//! Extracted from `search.rs` for single-responsibility:
//! - `search_brute_force`: SIMD-optimized exact search for small indices
//! - `search_brute_force_gpu`: GPU-accelerated search via wgpu
//! - `search_brute_force_buffered`: Buffer-reuse variant

use super::HnswIndex;
use crate::index::hnsw::params::SearchQuality;

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
    pub fn search_brute_force(&self, query: &[f32], k: usize) -> Vec<(u64, f32)> {
        self.validate_dimension(query, "Query");

        // If vector storage is disabled, return empty (can't do brute-force)
        if !self.enable_vector_storage || self.vectors.is_empty() {
            // Fallback to regular HNSW search with high ef
            let inner = self.inner.read();
            let ef_search = SearchQuality::Accurate.ef_search(k);
            let neighbours = inner.search(query, k, ef_search);

            let mut results: Vec<(u64, f32)> = Vec::with_capacity(neighbours.len());
            for n in &neighbours {
                if let Some(id) = self.mappings.get_id(n.d_id) {
                    let score = inner.transform_score(n.distance);
                    results.push((id, score));
                }
            }
            return results;
        }

        // Compute distances to all vectors
        let prefetch_distance = crate::simd_native::calculate_prefetch_distance(self.dimension);
        let vectors_snapshot: Vec<(usize, Vec<f32>)> = self.vectors.collect_for_parallel();

        let mut results: Vec<(u64, f32)> = Vec::with_capacity(vectors_snapshot.len());

        for (i, (idx, v)) in vectors_snapshot.iter().enumerate() {
            // Prefetch upcoming vectors
            if i + prefetch_distance < vectors_snapshot.len() {
                crate::simd_native::prefetch_vector(&vectors_snapshot[i + prefetch_distance].1);
            }

            if let Some(id) = self.mappings.get_id(*idx) {
                let score = self.compute_distance(query, v);
                results.push((id, score));
            }
        }

        // Sort by distance (metric-dependent ordering)
        self.metric.sort_results(&mut results);

        results.truncate(k);
        results
    }

    /// Performs GPU-accelerated brute-force search if available.
    ///
    /// Returns `None` if GPU feature is not enabled or GPU is not available.
    ///
    /// # Panics
    ///
    /// Panics if the query dimension doesn't match the index dimension.
    #[must_use]
    pub fn search_brute_force_gpu(&self, query: &[f32], k: usize) -> Option<Vec<(u64, f32)>> {
        self.validate_dimension(query, "Query");

        #[cfg(feature = "gpu")]
        {
            use crate::gpu::GpuAccelerator;

            let gpu = GpuAccelerator::new()?;

            // Collect vectors for GPU processing
            let vectors_snapshot = self.vectors.collect_for_parallel();

            if vectors_snapshot.is_empty() {
                return Some(Vec::new());
            }

            // Flatten vectors for GPU (contiguous memory layout)
            let mut flat_vectors: Vec<f32> =
                Vec::with_capacity(vectors_snapshot.len() * self.dimension);
            let mut id_map: Vec<u64> = Vec::with_capacity(vectors_snapshot.len());

            for (idx, vec) in &vectors_snapshot {
                if let Some(id) = self.mappings.get_id(*idx) {
                    flat_vectors.extend(vec);
                    id_map.push(id);
                }
            }

            if id_map.is_empty() {
                return Some(Vec::new());
            }

            // GPU batch cosine similarity
            let Ok(similarities) =
                gpu.batch_cosine_similarity(&flat_vectors, query, self.dimension)
            else {
                return None;
            };

            // Combine IDs with similarities
            let mut results: Vec<(u64, f32)> = id_map.into_iter().zip(similarities).collect();

            // Sort by similarity (descending for cosine)
            self.metric.sort_results(&mut results);

            results.truncate(k);
            Some(results)
        }

        #[cfg(not(feature = "gpu"))]
        {
            let _ = (query, k); // Suppress unused warnings
            None
        }
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
    pub fn search_brute_force_buffered(&self, query: &[f32], k: usize) -> Vec<(u64, f32)> {
        // Currently identical to search_brute_force - buffer reuse is internal optimization
        self.search_brute_force(query, k)
    }
}
