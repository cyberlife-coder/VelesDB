//! VectorIndex trait implementation for HnswIndex.

use super::HnswIndex;
use crate::distance::DistanceMetric;
use crate::index::hnsw::params::SearchQuality;
use crate::index::VectorIndex;
use crate::scored_result::ScoredResult;
use crate::validation::validate_dimension_match;

impl VectorIndex for HnswIndex {
    /// Inserts a vector, logging and silently dropping dimension mismatches.
    ///
    /// Invariant: validate dimension BEFORE `upsert_mapping` to prevent
    /// orphaned mappings on error. See `batch.rs` Phase Ordering comment.
    ///
    /// Callers that need error propagation should use
    /// [`HnswIndex::insert_batch_parallel`] which returns `Result`.
    #[inline]
    fn insert(&self, id: u64, vector: &[f32]) {
        if let Err(e) = validate_dimension_match(self.dimension, vector.len()) {
            tracing::error!("VectorIndex::insert dimension error for id={id}: {e}");
            return;
        }

        let result = self.upsert_mapping(id);
        self.insert_and_correct_mapping(id, vector, &result);
    }

    fn search(&self, query: &[f32], k: usize) -> Vec<ScoredResult> {
        match self.search_with_quality(query, k, SearchQuality::Balanced) {
            Ok(results) => results,
            Err(e) => {
                tracing::error!("VectorIndex::search failed: {e}");
                Vec::new()
            }
        }
    }

    /// Performs a **soft delete** of the vector.
    ///
    /// Removes the ID from mappings and cleans up stored vector data.
    /// The HNSW graph node becomes a tombstone, filtered out during search.
    ///
    /// For workloads with many deletions, consider periodic `vacuum()` to
    /// rebuild the graph and reclaim memory.
    fn remove(&self, id: u64) -> bool {
        if let Some(old_idx) = self.mappings.remove(id) {
            if self.enable_vector_storage {
                self.vectors.remove(old_idx);
            }
            true
        } else {
            false
        }
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
