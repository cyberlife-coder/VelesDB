//! Search methods for HnswIndex.

use super::HnswIndex;
use crate::distance::DistanceMetric;
use crate::index::hnsw::params::SearchQuality;

impl HnswIndex {
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
            DistanceMetric::Cosine => crate::simd::cosine_similarity_fast(query, vector),
            DistanceMetric::Euclidean => crate::simd::euclidean_distance_fast(query, vector),
            DistanceMetric::DotProduct => crate::simd::dot_product_fast(query, vector),
            DistanceMetric::Hamming => crate::simd::hamming_distance_fast(query, vector),
            DistanceMetric::Jaccard => crate::simd::jaccard_similarity_fast(query, vector),
        }
    }

    /// Searches for the k nearest neighbors with a specific quality profile.
    ///
    /// # Arguments
    ///
    /// * `query` - The query vector
    /// * `k` - Number of nearest neighbors to return
    /// * `quality` - Search quality profile controlling recall/latency tradeoff
    ///
    /// # Quality Profiles
    ///
    /// - `Fast`: ~92% recall, lowest latency
    /// - `Balanced`: ~99% recall, good tradeoff (default)
    /// - `Accurate`: ~100% recall, high precision
    /// - `Perfect`: 100% recall guaranteed via SIMD re-ranking
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
    ) -> Vec<(u64, f32)> {
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
        let inner = self.inner.read();

        // RF-1: Using HnswInner methods for search and score transformation
        let neighbours = inner.search(query, k, ef_search);
        let mut results: Vec<(u64, f32)> = Vec::with_capacity(neighbours.len());

        for n in &neighbours {
            if let Some(id) = self.mappings.get_id(n.d_id) {
                let score = inner.transform_score(n.distance);
                results.push((id, score));
            }
        }

        results
    }

    /// Searches with SIMD-based re-ranking for improved precision.
    ///
    /// This method first retrieves `rerank_k` candidates using the HNSW index,
    /// then re-ranks them using our SIMD-optimized distance functions for
    /// exact distance computation, returning the top `k` results.
    ///
    /// # Arguments
    ///
    /// * `query` - The query vector
    /// * `k` - Number of nearest neighbors to return
    /// * `rerank_k` - Number of candidates to retrieve before re-ranking (should be > k)
    ///
    /// # Returns
    ///
    /// Vector of (id, distance) tuples, sorted by similarity.
    /// For Cosine/DotProduct: higher is better (descending order).
    /// For Euclidean: lower is better (ascending order).
    ///
    /// # Panics
    ///
    /// Panics if the query dimension doesn't match the index dimension.
    #[must_use]
    pub fn search_with_rerank(&self, query: &[f32], k: usize, rerank_k: usize) -> Vec<(u64, f32)> {
        self.validate_dimension(query, "Query");

        // 1. Get candidates from HNSW (fast approximate search)
        let candidates = self.search_with_quality(query, rerank_k, SearchQuality::Accurate);

        if candidates.is_empty() {
            return Vec::new();
        }

        // 2. Re-rank using SIMD-optimized exact distance computation
        // EPIC-A.2: Collect candidate vectors from ShardedVectors for re-ranking
        let candidate_vectors: Vec<(u64, usize, Vec<f32>)> = candidates
            .iter()
            .filter_map(|(id, _)| {
                let idx = self.mappings.get_idx(*id)?;
                let vec = self.vectors.get(idx)?;
                Some((*id, idx, vec))
            })
            .collect();

        // Perf TS-CORE-001: Adaptive prefetch distance based on vector size
        let prefetch_distance = crate::simd::calculate_prefetch_distance(self.dimension);
        let mut reranked: Vec<(u64, f32)> = Vec::with_capacity(candidate_vectors.len());

        for (i, (id, _idx, v)) in candidate_vectors.iter().enumerate() {
            // Prefetch upcoming vectors (P1 optimization on local snapshot)
            if i + prefetch_distance < candidate_vectors.len() {
                crate::simd::prefetch_vector(&candidate_vectors[i + prefetch_distance].2);
            }

            // Compute exact distance using SIMD-optimized function
            let exact_dist = self.compute_distance(query, v);

            reranked.push((*id, exact_dist));
        }

        // 3. Sort by distance (metric-dependent ordering)
        self.metric.sort_results(&mut reranked);

        reranked.truncate(k);
        reranked
    }

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
        let prefetch_distance = crate::simd::calculate_prefetch_distance(self.dimension);
        let vectors_snapshot: Vec<(usize, Vec<f32>)> = self.vectors.collect_for_parallel();

        let mut results: Vec<(u64, f32)> = Vec::with_capacity(vectors_snapshot.len());

        for (i, (idx, v)) in vectors_snapshot.iter().enumerate() {
            // Prefetch upcoming vectors
            if i + prefetch_distance < vectors_snapshot.len() {
                crate::simd::prefetch_vector(&vectors_snapshot[i + prefetch_distance].1);
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
            let similarities = gpu.batch_cosine_similarity(&flat_vectors, query, self.dimension);

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

    /// Searches with SIMD-based re-ranking using a custom quality for initial search.
    ///
    /// Similar to `search_with_rerank` but allows specifying the quality profile
    /// for the initial HNSW search phase.
    ///
    /// # Arguments
    ///
    /// * `query` - The query vector
    /// * `k` - Number of nearest neighbors to return
    /// * `rerank_k` - Number of candidates to retrieve before re-ranking
    /// * `initial_quality` - Quality profile for initial HNSW search
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
    ) -> Vec<(u64, f32)> {
        self.validate_dimension(query, "Query");

        // 1. Get candidates from HNSW with specified quality
        // Avoid recursion if initial_quality is Perfect
        let actual_quality = if matches!(initial_quality, SearchQuality::Perfect) {
            SearchQuality::Accurate
        } else {
            initial_quality
        };
        let candidates = self.search_with_quality(query, rerank_k, actual_quality);

        if candidates.is_empty() {
            return Vec::new();
        }

        // 2. Re-rank using SIMD-optimized exact distance computation
        // EPIC-A.2: Collect candidate vectors from ShardedVectors
        let candidate_vectors: Vec<(u64, usize, Vec<f32>)> = candidates
            .iter()
            .filter_map(|(id, _)| {
                let idx = self.mappings.get_idx(*id)?;
                let vec = self.vectors.get(idx)?;
                Some((*id, idx, vec))
            })
            .collect();

        let prefetch_distance = crate::simd::calculate_prefetch_distance(self.dimension);
        let mut reranked: Vec<(u64, f32)> = Vec::with_capacity(candidate_vectors.len());

        for (i, (id, _idx, v)) in candidate_vectors.iter().enumerate() {
            // Prefetch upcoming vectors
            if i + prefetch_distance < candidate_vectors.len() {
                crate::simd::prefetch_vector(&candidate_vectors[i + prefetch_distance].2);
            }

            // Compute exact distance
            let exact_dist = self.compute_distance(query, v);

            reranked.push((*id, exact_dist));
        }

        // 3. Sort by distance (metric-dependent ordering)
        self.metric.sort_results(&mut reranked);

        reranked.truncate(k);
        reranked
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

    /// Sets the index to searching mode after bulk insertions.
    ///
    /// This is required by `hnsw_rs` after parallel insertions to ensure
    /// correct search results. Call this after finishing all insertions
    /// and before performing searches.
    ///
    /// For single-threaded sequential insertions, this is typically not needed,
    /// but it's good practice to call it anyway before benchmarks.
    pub fn set_searching_mode(&self) {
        // RF-1: Using HnswInner method
        let mut inner = self.inner.write();
        inner.set_searching_mode(true);
    }
}
