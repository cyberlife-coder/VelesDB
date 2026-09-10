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
    /// The dimension is checked before the graph sees the vector, and the id
    /// is mapped only to the slot the graph then gives it (#2246).
    ///
    /// [`HnswIndex::insert_batch_parallel`] does not propagate an error either:
    /// it returns how many vectors it inserted and logs why it refused a batch.
    #[inline]
    fn insert(&self, id: u64, vector: &[f32]) {
        if let Err(e) = validate_dimension_match(self.dimension, vector.len()) {
            tracing::error!("VectorIndex::insert dimension error for id={id}: {e}");
            return;
        }

        self.insert_and_assign(id, vector);
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
    /// Delegates to the inherent [`HnswIndex::remove`] — see that method's
    /// rustdoc for semantics (tombstoning, sidecar cleanup, vacuum guidance).
    #[inline]
    fn remove(&self, id: u64) -> bool {
        HnswIndex::remove(self, id)
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
