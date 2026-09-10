//! Vacuum and maintenance operations for HnswIndex.

use super::{HnswIndex, HnswInner};
use crate::index::hnsw::params::HnswParams;
use crate::index::hnsw::sharded_mappings::SlotsPinned;
use std::mem::ManuallyDrop;

/// Errors that can occur during vacuum operations.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum VacuumError {
    /// The index's exact-distance features are off
    /// (`enable_vector_storage = false`), and vacuum needs them to rebuild it.
    #[error("Cannot vacuum: exact-distance features are off (build the index with new(), not new_fast_insert())")]
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
        // Every slot below next_idx that no id names is dead: deleted,
        // replaced, or a direct write its graph insert superseded.
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
    /// - **Writes during a vacuum are lost to the index** (#2262): an insert or
    ///   delete that lands after step 1's snapshot reaches the old graph, which
    ///   the swap drops, and the mapping rebuild re-creates the snapshot — so a
    ///   new id stays out of search and a deleted one comes back until the next
    ///   open's recovery. Searches themselves are safe: the swap and the mapping
    ///   rebuild happen under one write lock.
    /// - Requires exact-distance features (`enable_vector_storage = true`); the
    ///   graph stores vectors either way
    ///
    /// # Returns
    ///
    /// - `Ok(count)` - Number of vectors in the rebuilt index
    /// - `Err` - If exact-distance features are off or the rebuild fails
    ///
    /// # Errors
    ///
    /// Returns `VacuumError::VectorStorageDisabled` if the index was created
    /// with `new_fast_insert()`, which turns its exact-distance features off.
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
        let (new_inner, slots) = self.build_vacuum_replacement(&active_vectors)?;

        // 5-6. Swap in the new graph and rebuild the mappings under one write
        // lock: until the rebuild they name the old graph's slots, and a
        // search in between would resolve ids against the wrong vectors
        // (#2246).
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

            // Each id follows the slot its vector was given in the new graph.
            // ShardedMappings uses interior mutability, so we clear and
            // repopulate in place.
            self.mappings.clear();
            debug_assert_eq!(
                active_vectors.len(),
                slots.len(),
                "the rebuild returns one slot per vector"
            );
            for ((id, _vec), slot) in active_vectors.iter().zip(slots) {
                let previous = self
                    .mappings
                    .assign(*id, slot, SlotsPinned::by_write(&inner_guard));
                debug_assert!(
                    previous.is_none(),
                    "Vacuum invariant violated: duplicate id {id} while rebuilding mappings"
                );
            }
            drop(inner_guard);
        }

        Ok(count)
    }

    /// Builds the replacement inner index for [`Self::vacuum`].
    ///
    /// Returns it with the slot each of `active_vectors` was given, in order.
    ///
    /// Creates a new graph with auto-tuned parameters, **preserving the
    /// current backend storage mode** (a RaBitQ index must not silently
    /// downgrade to the Standard f32 backend on vacuum), inserts the active
    /// vectors, and re-installs the trained RaBitQ quantizer when present
    /// (re-encodes the compacted vectors in NodeId order — without this, a
    /// vacuumed RaBitQ index would fall back to f32 search until the next
    /// collection open).
    /// Where the replacement graph's f32 arena belongs, given what it will
    /// become.
    ///
    /// Keyed on the *target* mode, not the mode the replacement is built
    /// with: `build_vacuum_replacement` deliberately builds through a
    /// `Full` backend and promotes afterwards, and the promotion moves the
    /// graph as-is. A heap arena chosen here would therefore survive the
    /// promotion and quietly undo the mapping on every compaction (#2112).
    fn arena_dir_for(&self, target_mode: crate::StorageMode) -> Option<&std::path::Path> {
        match target_mode {
            crate::StorageMode::RaBitQ | crate::StorageMode::SQ8 => self.arena_dir.as_deref(),
            _ => None,
        }
    }

    fn build_vacuum_replacement(
        &self,
        active_vectors: &[(u64, Vec<f32>)],
    ) -> Result<(HnswInner, Vec<usize>), VacuumError> {
        let params = HnswParams::auto(self.dimension);
        let target_mode = self.inner.read().storage_mode();
        // Always rebuild through a Standard backend: inserting via a RaBitQ
        // backend would lazily train a throwaway quantizer at the sample
        // threshold (then re-encode everything a second time on install) —
        // and would silently SELF-train an untrained collection from
        // compaction order. The graph is promoted afterwards.
        //
        // The arena dir still has to be `target_mode`'s, not `Full`'s: the
        // graph built here is the one `promote_to_*` hands to the quantized
        // wrapper, so if it were built heap-backed the promotion would carry
        // a heap arena and every vacuum would silently undo the mapping
        // (#2112). Building it mapped up front is safe precisely because
        // arenas are per-instance — the old index is still serving reads from
        // its own file, and the two never contend.
        let new_inner = HnswInner::build(&crate::index::hnsw::native_inner::InnerBuild {
            metric: self.metric,
            max_connections: params.max_connections,
            max_elements: active_vectors.len().max(1000),
            ef_construction: params.ef_construction,
            dimension: self.dimension,
            storage_mode: crate::StorageMode::Full,
            alpha: params.alpha,
            arena_dir: self.arena_dir_for(target_mode),
        })
        .map_err(|e| VacuumError::RebuildFailed(e.to_string()))?;

        let vectors: Vec<&[f32]> = active_vectors
            .iter()
            .map(|(_id, vec)| vec.as_slice())
            .collect();
        let slots = new_inner
            .parallel_insert(&vectors)
            .map_err(|e| VacuumError::RebuildFailed(e.to_string()))?;

        Ok((self.promote_replacement(new_inner, target_mode)?, slots))
    }

    /// Promotes a freshly rebuilt `Full` graph to `target_mode` and carries
    /// the old index's trained quantizer onto it.
    ///
    /// Split from `build_vacuum_replacement` to keep that function inside the
    /// complexity gate; the two halves are genuinely separate steps — build
    /// the graph, then decide what it becomes.
    fn promote_replacement(
        &self,
        new_inner: HnswInner,
        target_mode: crate::StorageMode,
    ) -> Result<HnswInner, VacuumError> {
        match target_mode {
            crate::StorageMode::RaBitQ => {
                let new_inner = new_inner.promote_to_rabitq(self.dimension);
                // Bound before the `if let`: the old-index read guard would
                // otherwise be held across `install_trained_rabitq`'s full
                // O(n*d) re-encode pass. Carrying the quantizer out first
                // releases it immediately (#2109's lock-guard family).
                #[cfg(feature = "persistence")]
                let carried = self.inner.read().rabitq_quantizer();
                #[cfg(feature = "persistence")]
                if let Some(rabitq) = carried {
                    // Single encode pass with the carried-over quantizer; an
                    // untrained collection stays untrained (no state change).
                    new_inner
                        .install_trained_rabitq(rabitq)
                        .map_err(|e| VacuumError::RebuildFailed(e.to_string()))?;
                }
                Ok(new_inner)
            }
            crate::StorageMode::SQ8 => {
                let new_inner = new_inner.promote_to_sq8(self.dimension);
                // Same binding, same reason: `install_trained_sq8` re-encodes
                // every vector, and the guard must not span it.
                #[cfg(feature = "persistence")]
                let carried = self.inner.read().sq8_quantizer();
                #[cfg(feature = "persistence")]
                if let Some(quantizer) = carried {
                    new_inner
                        .install_trained_sq8(quantizer)
                        .map_err(|e| VacuumError::RebuildFailed(e.to_string()))?;
                }
                Ok(new_inner)
            }
            _ => Ok(new_inner),
        }
    }
}
