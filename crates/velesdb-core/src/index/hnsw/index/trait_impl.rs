//! VectorIndex trait implementation for HnswIndex.

use super::HnswIndex;
use crate::distance::DistanceMetric;
use crate::index::hnsw::params::SearchQuality;
use crate::index::VectorIndex;

impl VectorIndex for HnswIndex {
    #[inline]
    fn insert(&self, id: u64, vector: &[f32]) {
        // Inline validation for hot path performance
        assert_eq!(
            vector.len(),
            self.dimension,
            "Vector dimension mismatch: expected {}, got {}",
            self.dimension,
            vector.len()
        );

        // Register the ID and get internal index with ShardedMappings
        // Check if ID already exists - hnsw_rs doesn't support updates!
        // register() returns None if ID already exists
        let Some(idx) = self.mappings.register(id) else {
            return; // ID already exists, skip insertion
        };

        // Insert into HNSW index (RF-1: using HnswInner method)
        // Perf: Minimize lock hold time by not explicitly dropping
        self.inner.write().insert((vector, idx));

        // Perf: Conditionally store vector for SIMD re-ranking
        // When disabled, saves ~50% memory and ~2x insert speed
        if self.enable_vector_storage {
            self.vectors.insert(idx, vector);
        }
    }

    fn search(&self, query: &[f32], k: usize) -> Vec<(u64, f32)> {
        // Perf: Use Balanced quality for best latency/recall tradeoff
        // ef_search=128 provides ~95% recall with minimal latency
        self.search_with_quality(query, k, SearchQuality::Balanced)
    }

    /// Performs a **soft delete** of the vector.
    ///
    /// # Important
    ///
    /// This removes the ID from the mappings but **does NOT remove the vector
    /// from the HNSW graph** (`hnsw_rs` doesn't support true deletion).
    /// The vector will no longer appear in search results, but memory is not freed.
    ///
    /// For workloads with many deletions, consider periodic index rebuilding
    /// to reclaim memory and maintain optimal graph structure.
    fn remove(&self, id: u64) -> bool {
        // EPIC-A.1: Lock-free removal with ShardedMappings
        // Soft delete: vector remains in HNSW graph but is excluded from results
        self.mappings.remove(id).is_some()
    }

    fn len(&self) -> usize {
        self.mappings.len()
    }

    fn dimension(&self) -> usize {
        self.dimension
    }

    fn metric(&self) -> DistanceMetric {
        self.metric
    }
}
