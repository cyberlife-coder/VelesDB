//! Vacuum and maintenance operations for HnswIndex.

use super::{HnswIndex, HnswInner};
use crate::index::hnsw::params::HnswParams;
use std::mem::ManuallyDrop;

/// Errors that can occur during vacuum operations.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum VacuumError {
    /// Vector storage is disabled, cannot rebuild index.
    #[error("Cannot vacuum: vector storage is disabled (use new() instead of new_fast_insert())")]
    VectorStorageDisabled,
    /// Index rebuild failed (allocation or insertion error).
    #[error("Vacuum rebuild failed: {0}")]
    RebuildFailed(String),
}

impl HnswIndex {
    /// Returns the number of tombstones (soft-deleted entries) in the index.
    ///
    /// Tombstones are entries that have been removed from mappings but still
    /// exist in the underlying HNSW graph. High tombstone count degrades
    /// search performance.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let index = HnswIndex::new(128, DistanceMetric::Cosine);
    /// // Insert and delete some vectors...
    /// if index.tombstone_ratio() > 0.2 {
    ///     index.needs_vacuum(); // Consider rebuilding
    /// }
    /// ```
    #[must_use]
    pub fn tombstone_count(&self) -> usize {
        // Total inserted = next_idx in mappings (monotonic counter)
        // Active = mappings.len()
        // Tombstones = Total - Active
        let total_inserted = self.mappings.next_idx();
        let active = self.mappings.len();
        total_inserted.saturating_sub(active)
    }

    /// Returns the tombstone ratio (0.0 = clean, 1.0 = 100% deleted).
    ///
    /// Use this to decide when to trigger a vacuum/rebuild operation.
    /// A ratio > 0.2 (20%) is a reasonable threshold for considering vacuum.
    #[must_use]
    #[allow(clippy::cast_precision_loss)] // Acceptable precision loss for ratio calculation
    pub fn tombstone_ratio(&self) -> f64 {
        let total = self.mappings.next_idx();
        if total == 0 {
            return 0.0;
        }
        let tombstones = self.tombstone_count();
        tombstones as f64 / total as f64
    }

    /// Returns true if the index has significant fragmentation and would
    /// benefit from a vacuum/rebuild operation.
    ///
    /// Current threshold: 20% tombstones
    #[must_use]
    pub fn needs_vacuum(&self) -> bool {
        self.tombstone_ratio() > 0.2
    }

    /// Rebuilds the HNSW index, removing all tombstones.
    ///
    /// This creates a new HNSW graph containing only the active vectors,
    /// eliminating fragmentation and improving search performance.
    ///
    /// # Important
    ///
    /// - This operation is **blocking** and may take significant time for large indices
    /// - **Consistency**: between the graph swap and mappings rebuild, concurrent
    ///   searches may return incomplete results. Callers should avoid concurrent
    ///   queries during vacuum or accept transient result gaps.
    /// - Requires `enable_vector_storage = true` (vectors must be stored)
    ///
    /// # Returns
    ///
    /// - `Ok(count)` - Number of vectors in the rebuilt index
    /// - `Err` - If vector storage is disabled or rebuild fails
    ///
    /// # Errors
    ///
    /// Returns `VacuumError::VectorStorageDisabled` if the index was created
    /// with `new_fast_insert()` mode, which disables vector storage.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let index = HnswIndex::new(128, DistanceMetric::Cosine);
    /// // ... insert and delete many vectors ...
    ///
    /// if index.needs_vacuum() {
    ///     let count = index.vacuum()?;
    ///     println!("Rebuilt index with {} vectors", count);
    /// }
    /// ```
    pub fn vacuum(&self) -> Result<usize, VacuumError> {
        if !self.enable_vector_storage {
            return Err(VacuumError::VectorStorageDisabled);
        }

        // 1. Collect all active vectors: snapshot live mappings and read each
        // vector from the graph's ContiguousVectors (single source of truth).
        // For cosine indices these are the pre-normalized vectors; re-insertion
        // re-normalizes, which is idempotent up to f32 rounding.
        let active_vectors: Vec<(u64, Vec<f32>)> = {
            let inner = self.inner.read();
            inner.with_contiguous_vectors(|vectors| {
                self.mappings
                    .iter()
                    .filter_map(|(id, idx)| vectors.get(idx).map(|vec| (id, vec.to_vec())))
                    .collect()
            })
        };

        let count = active_vectors.len();

        if count == 0 {
            return Ok(0);
        }

        // 2-4. Rebuild a fresh inner index from the active vectors,
        // preserving the backend storage mode and trained quantizer.
        let new_inner = self.build_vacuum_replacement(&active_vectors)?;

        // 5. Atomic swap (replace old with new)
        {
            let mut inner_guard = self.inner.write();
            // SAFETY: ManuallyDrop::drop is safe when exclusive ownership is guaranteed.
            // - Condition 1: We hold exclusive write lock on inner_guard (no other access possible)
            // - Condition 2: This is called exactly once before replacement (no double-drop)
            // - Condition 3: The old value is immediately replaced with new_inner (no use-after-free)
            // SAFETY: Explicit drop required before assignment to ManuallyDrop field.
            unsafe {
                ManuallyDrop::drop(&mut *inner_guard);
            }
            // Replace with new
            *inner_guard = ManuallyDrop::new(new_inner);
        }

        // 6. Rebuild mappings to match the compacted graph (sequential
        // indices 0..count, same order as `refs_for_hnsw` above).
        // Note: ShardedMappings uses interior mutability, so we clear and
        // repopulate in place.
        self.mappings.clear();

        for (id, _vec) in active_vectors {
            if self.mappings.register(id).is_none() {
                debug_assert!(
                    false,
                    "Vacuum invariant violated: duplicate id encountered while rebuilding mappings"
                );
            }
        }

        Ok(count)
    }

    /// Builds the replacement inner index for [`Self::vacuum`].
    ///
    /// Creates a new graph with auto-tuned parameters, **preserving the
    /// current backend storage mode** (a RaBitQ index must not silently
    /// downgrade to the Standard f32 backend on vacuum), inserts the active
    /// vectors, and re-installs the trained RaBitQ quantizer when present
    /// (re-encodes the compacted vectors in NodeId order — without this, a
    /// vacuumed RaBitQ index would fall back to f32 search until the next
    /// collection open).
    fn build_vacuum_replacement(
        &self,
        active_vectors: &[(u64, Vec<f32>)],
    ) -> Result<HnswInner, VacuumError> {
        let params = HnswParams::auto(self.dimension);
        let target_mode = self.inner.read().storage_mode();
        // Always rebuild through a Standard backend: inserting via a RaBitQ
        // backend would lazily train a throwaway quantizer at the sample
        // threshold (then re-encode everything a second time on install) —
        // and would silently SELF-train an untrained collection from
        // compaction order. The graph is promoted afterwards.
        let new_inner = HnswInner::new_with_storage_mode(
            self.metric,
            params.max_connections,
            active_vectors.len().max(1000), // max_elements with reasonable minimum
            params.ef_construction,
            self.dimension,
            crate::StorageMode::Full,
        )
        .map_err(|e| VacuumError::RebuildFailed(e.to_string()))?;

        // Insertion references: idx = sequential, matches graph allocation.
        let refs_for_hnsw: Vec<(&[f32], usize)> = active_vectors
            .iter()
            .enumerate()
            .map(|(idx, (_id, vec))| (vec.as_slice(), idx))
            .collect();

        new_inner
            .parallel_insert(&refs_for_hnsw)
            .map_err(|e| VacuumError::RebuildFailed(e.to_string()))?;

        if target_mode != crate::StorageMode::RaBitQ {
            return Ok(new_inner);
        }
        let new_inner = new_inner.promote_to_rabitq(self.dimension);
        #[cfg(feature = "persistence")]
        if let Some(rabitq) = self.inner.read().rabitq_quantizer() {
            // Single encode pass with the carried-over quantizer; an
            // untrained collection stays untrained (no state change).
            new_inner
                .install_trained_rabitq(rabitq)
                .map_err(|e| VacuumError::RebuildFailed(e.to_string()))?;
        }

        Ok(new_inner)
    }
}
