//! Direct vector writer for bulk insert bypass of `ShardedVectors`.
//!
//! During bulk insert, the standard path writes each vector through
//! `ShardedVectors` (16 shards × `RwLock` × `FxHash` per vector).
//! `DirectVectorWriter` bypasses this overhead by writing directly to
//! `ContiguousVectors` via the `NativeHnsw` inner graph, deferring
//! `ShardedVectors` synchronization until after HNSW indexing completes.

use super::index::HnswIndex;
use super::upsert::{self, UpsertResult};
use crate::validation::validate_dimension_match;

/// Writes vectors directly to `ContiguousVectors`, bypassing `ShardedVectors`.
///
/// Used exclusively during `upsert_bulk` to eliminate per-vector sharding
/// overhead (16 shards × `RwLock` × `FxHash`). After indexing completes,
/// call [`sync_to_sharded`](Self::sync_to_sharded) to populate
/// `ShardedVectors` for SIMD re-ranking and brute-force search.
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
    /// Does **not** write to `ShardedVectors` — call [`sync_to_sharded`](Self::sync_to_sharded)
    /// after HNSW indexing to populate the sidecar storage.
    ///
    /// When `enable_vector_storage` is `false`, only mappings are registered
    /// (no vector data is written to `ContiguousVectors` sidecar).
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
        let results = upsert::upsert_mapping_batch(
            &self.hnsw_index.mappings,
            &self.hnsw_index.vectors,
            self.hnsw_index.enable_vector_storage,
            &ids,
        );

        // Write vectors to ContiguousVectors inside NativeHnsw (bypass ShardedVectors).
        // When enable_vector_storage is false, the graph's own ContiguousVectors
        // is still populated by the HNSW insert path, so we skip sidecar writes.
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
            storage.ensure_capacity(max_idx + 1)?;

            for ((_, vector), result) in vectors.iter().zip(results.iter()) {
                storage.insert_at(result.idx, vector)?;
            }
            Ok(())
        })
    }

    /// Synchronizes `ContiguousVectors` → `ShardedVectors` after HNSW indexing.
    ///
    /// For each `UpsertResult`, reads the vector from `ContiguousVectors` and
    /// inserts it into `ShardedVectors`. No-op when `enable_vector_storage`
    /// is `false`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::Error::Internal`] if a vector cannot be read from
    /// `ContiguousVectors` (indicates a bug in the write path).
    ///
    /// [`crate::error::Error::Internal`]: crate::error::Error::Internal
    #[allow(clippy::unnecessary_wraps)] // Returns Result for API consistency with Task 4
    pub(crate) fn sync_to_sharded(&self, results: &[UpsertResult]) -> crate::error::Result<()> {
        if !self.hnsw_index.enable_vector_storage || results.is_empty() {
            return Ok(());
        }

        let inner = self.hnsw_index.inner.read();
        let pairs: Vec<(usize, Vec<f32>)> = inner.with_contiguous_vectors_read(|storage| {
            let mut out = Vec::with_capacity(results.len());
            for result in results {
                if let Some(vec) = storage.get(result.idx) {
                    out.push((result.idx, vec.to_vec()));
                }
            }
            out
        });

        self.hnsw_index.vectors.insert_batch(pairs);
        Ok(())
    }
}
