//! Direct vector writer for bulk insert.
//!
//! During bulk insert with deferred HNSW construction, vectors must be
//! visible to rerank/brute-force before graph indexing completes.
//! `DirectVectorWriter` writes vector data straight into the graph's
//! `ContiguousVectors` (the single vector store) and maps each id to the slot
//! its vector got.

use super::index::HnswIndex;
use super::upsert::UpsertResult;
use crate::validation::validate_dimension_match;

/// Writes vectors directly to the graph's `ContiguousVectors`.
///
/// Used exclusively during `upsert_bulk` so vectors are immediately
/// available for SIMD re-ranking and brute-force search while HNSW graph
/// construction is deferred to `AsyncIndexBuilder`.
#[cfg_attr(not(feature = "persistence"), allow(dead_code))] // Reason: only the persistence-gated bulk path (collection::core::crud_bulk) writes directly
pub(crate) struct DirectVectorWriter<'a> {
    hnsw_index: &'a HnswIndex,
}

#[cfg_attr(not(feature = "persistence"), allow(dead_code))] // Reason: only the persistence-gated bulk path (collection::core::crud_bulk) writes directly
impl<'a> DirectVectorWriter<'a> {
    /// Creates a new direct writer for the given `HnswIndex`.
    #[must_use]
    pub(crate) fn new(hnsw_index: &'a HnswIndex) -> Self {
        Self { hnsw_index }
    }

    /// Inserts a batch of vectors directly into `ContiguousVectors`, then maps
    /// each id to the slot its vector got (#2246).
    ///
    /// The vectors are appended under one write lock on the graph's arena —
    /// the same lock every graph insert allocates under — and each id is then
    /// mapped to the slot its vector landed in. The mapping never predicts a
    /// slot, so a concurrent graph insert can no longer take the slot this
    /// batch writes, nor this batch overwrite a vector the graph placed.
    ///
    /// When `enable_vector_storage` is `false` (the index's exact-distance
    /// features are off) this writer places no vector, so it has no slot to
    /// map: nothing is written or registered here, and the deferred HNSW
    /// insert places each vector and maps its id.
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::Error::DimensionMismatch`] if any vector has wrong dimension.
    /// Returns [`Error::AllocationFailed`] if `ContiguousVectors` cannot grow.
    /// On error, state is unchanged (all-or-nothing validation).
    ///
    /// [`crate::error::Error::DimensionMismatch`]: crate::error::Error::DimensionMismatch
    /// [`Error::AllocationFailed`]: crate::error::Error::AllocationFailed
    pub(crate) fn write_batch_direct(
        &self,
        vectors: &[(u64, &[f32])],
    ) -> crate::error::Result<Vec<UpsertResult>> {
        // Every dimension is checked before any mutation, whatever the
        // index's features: `# Errors` promises it.
        for (_, vector) in vectors {
            validate_dimension_match(self.hnsw_index.dimension, vector.len())?;
        }
        if vectors.is_empty() || !self.hnsw_index.enable_vector_storage {
            return Ok(Vec::new());
        }

        let refs: Vec<&[f32]> = vectors.iter().map(|&(_, vector)| vector).collect();
        // Held until every id is mapped: `reorder_for_locality` and `vacuum`
        // renumber slots under the write lock, so each slot placed here is still
        // its vector's when the mapping names it.
        let inner = self.hnsw_index.inner.read();
        let placed = inner.place_unlinked(&refs)?;
        let results = vectors
            .iter()
            .zip(placed)
            .map(|(&(id, _), placed)| UpsertResult {
                idx: placed.slot(),
                old_idx: self.hnsw_index.mappings.assign(id, placed),
            })
            .collect();
        drop(inner);
        Ok(results)
    }
}
