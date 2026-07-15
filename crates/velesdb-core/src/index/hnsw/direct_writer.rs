//! Direct vector writer for bulk insert.
//!
//! During bulk insert with deferred HNSW construction, vectors must be
//! visible to rerank/brute-force before graph indexing completes.
//! `DirectVectorWriter` registers ID mappings and writes vector data
//! straight into the graph's `ContiguousVectors` (the single vector store).

use super::index::HnswIndex;
use super::upsert::{self, UpsertResult};
use crate::validation::validate_dimension_match;

/// Writes vectors directly to the graph's `ContiguousVectors`.
///
/// Used exclusively during `upsert_bulk` so vectors are immediately
/// available for SIMD re-ranking and brute-force search while HNSW graph
/// construction is deferred to `AsyncIndexBuilder`.
#[allow(dead_code)] // Wired into Collection pipeline in Task 4
pub(crate) struct DirectVectorWriter<'a> {
    hnsw_index: &'a HnswIndex,
}

#[allow(dead_code)] // Wired into Collection pipeline in Task 4
impl<'a> DirectVectorWriter<'a> {
    /// Creates a new direct writer for the given `HnswIndex`.
    #[must_use]
    pub(crate) fn new(hnsw_index: &'a HnswIndex) -> Self {
        Self { hnsw_index }
    }

    /// Inserts a batch of vectors directly into `ContiguousVectors`.
    ///
    /// For each vector:
    /// 1. Registers the external ID via `ShardedMappings` (upsert semantics)
    /// 2. Writes the vector data to `ContiguousVectors` inside `NativeHnsw`
    ///
    /// When `enable_vector_storage` is `false`, only mappings are registered
    /// (the deferred HNSW insert path will populate `ContiguousVectors`).
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
        if vectors.is_empty() {
            return Ok(Vec::new());
        }

        // Validate ALL dimensions upfront before any mutation.
        for (_, vector) in vectors {
            validate_dimension_match(self.hnsw_index.dimension, vector.len())?;
        }

        // Register mappings (upsert semantics: replaces existing IDs).
        let ids: Vec<u64> = vectors.iter().map(|(id, _)| *id).collect();
        let results = upsert::upsert_mapping_batch(&self.hnsw_index.mappings, &ids);

        // Write vectors to ContiguousVectors inside NativeHnsw.
        // When enable_vector_storage is false, the graph's own ContiguousVectors
        // is still populated by the (deferred) HNSW insert path, so we skip the
        // direct write here.
        if self.hnsw_index.enable_vector_storage {
            self.write_to_contiguous(vectors, &results)?;
        }

        Ok(results)
    }

    /// Writes vector data into the `NativeHnsw` `ContiguousVectors` storage.
    ///
    /// Acquires the write lock once for the entire batch.
    fn write_to_contiguous(
        &self,
        vectors: &[(u64, &[f32])],
        results: &[UpsertResult],
    ) -> crate::error::Result<()> {
        let inner = self.hnsw_index.inner.read();
        inner.with_contiguous_vectors_mut(|storage| {
            // Ensure capacity for all new vectors.
            let max_idx = results.iter().map(|r| r.idx).max().unwrap_or(0);
            // #899 follow-up: use checked_add for consistency with the rest of
            // the allocation hardening — a wrapped `max_idx + 1` would otherwise
            // request capacity 0 and let `insert_at` write out of bounds.
            let required = max_idx.checked_add(1).ok_or_else(|| {
                crate::error::Error::AllocationFailed(format!(
                    "write_to_contiguous: max index {max_idx} + 1 overflows usize"
                ))
            })?;
            storage.ensure_capacity(required)?;

            for ((_, vector), result) in vectors.iter().zip(results.iter()) {
                storage.insert_at(result.idx, vector)?;
            }
            Ok(())
        })
    }
}
