//! Vacuum and maintenance operations for HnswIndex.

use super::{HnswIndex, HnswInner};
use crate::index::hnsw::native_inner::Placed;
use rustc_hash::FxHashMap;
use std::mem::ManuallyDrop;

/// Errors that can occur during vacuum operations.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum VacuumError {
    /// The index's exact-distance features are off
    /// (`enable_vector_storage = false`); vacuum refuses such an index.
    #[error("Cannot vacuum: exact-distance features are off (use new(), not new_fast_insert())")]
    VectorStorageDisabled,
    /// Index rebuild failed: an allocation or insertion error, or a rebuild
    /// whose slot count differs from its id count.
    #[error("Vacuum rebuild failed: {0}")]
    RebuildFailed(String),
}

/// A live id as `vacuum`'s snapshot saw it: the slot it held in the old graph
/// and the vector stored there.
struct Live {
    id: u64,
    slot: usize,
    vector: Vec<f32>,
}

/// `slots`, if the placement gave each of its `vectors` vectors one: a short
/// placement, once installed, would leave ids with no slot and nothing to fall
/// back to.
fn one_slot_each(slots: Vec<usize>, vectors: usize) -> Result<Vec<usize>, VacuumError> {
    if slots.len() == vectors {
        Ok(slots)
    } else {
        Err(VacuumError::RebuildFailed(format!(
            "the rebuild placed {} vectors for {vectors} ids",
            slots.len()
        )))
    }
}

/// Copies each `(id, slot)` of `written` from `old` into `new`: the vector at
/// `slot` in `old` is inserted into `new`, and `id` comes back with the slot it
/// got there.
///
/// # Errors
///
/// [`VacuumError::RebuildFailed`] if a slot holds no vector in `old`, or if
/// `new` refuses one.
fn carry_over(
    old: &HnswInner,
    new: &HnswInner,
    written: &[(u64, usize)],
) -> Result<Vec<(u64, usize)>, VacuumError> {
    let vectors: Option<Vec<Vec<f32>>> = old.with_contiguous_vectors(|arena| {
        written
            .iter()
            .map(|&(_, slot)| arena.get(slot).map(<[f32]>::to_vec))
            .collect()
    });
    let vectors = vectors.ok_or_else(|| {
        VacuumError::RebuildFailed("a mapped slot holds no vector in the old graph".into())
    })?;
    let refs: Vec<&[f32]> = vectors.iter().map(Vec::as_slice).collect();
    let slots = new
        .parallel_insert(&refs)
        .map_err(|e| VacuumError::RebuildFailed(e.to_string()))?;
    let slots = one_slot_each(slots, refs.len())?;
    Ok(written.iter().map(|&(id, _)| id).zip(slots).collect())
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
    /// - **Writes made during a vacuum survive it** (#2262). The rebuild works
    ///   from a snapshot and does not hold the graph lock while it inserts, so
    ///   searches and writes carry on against the old graph. The swap then
    ///   re-maps the ids mapped at that moment, not the snapshot's: an id
    ///   inserted or upserted since the snapshot has its vector copied from the
    ///   old graph into the new one, and an id deleted since stays deleted. One
    ///   write lock covers the swap, the re-map and those copies, so a search
    ///   never sees a half-built mapping; the more writes land during the
    ///   rebuild, the longer it is held.
    /// - [`Self::reorder_for_locality`] waits for a running vacuum, and a
    ///   vacuum for a running reorder: the two share one maintenance lock.
    ///   [`Self::save`] takes no part in it: during the rebuild it saves the
    ///   old graph, and the swap waits for its dump like for any read guard.
    /// - An index with no live id is rebuilt empty: every dead slot is
    ///   reclaimed, and the tombstone count reads 0.
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

        // Held to the end, and taken before any graph guard: nothing renumbers
        // a slot until the swap, so an id still on the slot the snapshot saw
        // has not been written since (the arena never hands a slot out twice).
        let _maintenance = self.maintenance.lock();

        // 1. Snapshot the live ids, each with its slot and its vector.
        let live = self.snapshot_live();
        let count = live.len();
        // Nothing to reclaim only when the graph holds no slot at all. With no
        // live id but dead slots, the rebuild still runs, into an empty graph:
        // returning here left every dead slot in place, and the tombstone
        // count asking for a vacuum that reclaimed none (#2262).
        if count == 0 && self.graph_vector_count() == 0 {
            return Ok(0);
        }

        // 2-4. Rebuild a fresh inner index from the snapshot, preserving the
        // backend storage mode and trained quantizer. No graph guard is held:
        // writers carry on against the old graph meanwhile.
        let (new_inner, slots) = self.build_vacuum_replacement(&live)?;
        // Checked before the swap, as `carry_over` checks its own placement.
        let slots = one_slot_each(slots, count)?;
        // Each snapshot id, with the slot it held and the slot it was rebuilt at.
        let rebuilt: FxHashMap<u64, (usize, usize)> = live
            .into_iter()
            .zip(slots)
            .map(|(live, slot)| (live.id, (live.slot, slot)))
            .collect();

        // 5-6. Swap in the new graph and rebuild the mappings under one write
        // lock: until the rebuild they name the old graph's slots, and a
        // search in between would resolve ids against the wrong vectors
        // (#2246). They are rebuilt from what is mapped now, not from the
        // snapshot, or every write made during the rebuild would be lost (#2262).
        let placed = {
            let mut inner_guard = self.inner.write();
            let placed = self.reconcile(&inner_guard, &new_inner, &rebuilt)?;
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

            // Each id follows the slot its vector has in the new graph, and
            // every slot of it no id comes to name counts as a tombstone: the
            // rebuild placed every snapshot id, and `reconcile` left out the
            // ones deleted since, wherever their slots fall (#2262).
            // ShardedMappings uses interior mutability, so we clear and
            // repopulate in place.
            let slots = inner_guard
                .with_contiguous_vectors(crate::perf_optimizations::ContiguousVectors::len);
            self.mappings.clear_for(slots);
            for &(id, slot) in &placed {
                let previous = self
                    .mappings
                    .assign(id, Placed::installed(&inner_guard, slot));
                debug_assert!(
                    previous.is_none(),
                    "Vacuum invariant violated: duplicate id {id} while rebuilding mappings"
                );
            }
            drop(inner_guard);
            placed.len()
        };

        Ok(placed)
    }

    /// Step 1 of [`Self::vacuum`]: every live id, with its slot and the vector
    /// stored there, read from the graph's `ContiguousVectors` (single source
    /// of truth). For cosine indices these are the pre-normalized vectors;
    /// re-insertion re-normalizes, which is idempotent up to f32 rounding.
    ///
    /// Writers hold the same read side and may run meanwhile, so the snapshot
    /// is not one instant: [`Self::reconcile`] settles whatever they change.
    fn snapshot_live(&self) -> Vec<Live> {
        let inner = self.inner.read();
        inner.with_contiguous_vectors(|vectors| {
            self.mappings
                .iter()
                .filter_map(|(id, slot)| {
                    let vector = vectors.get(slot)?.to_vec();
                    Some(Live { id, slot, vector })
                })
                .collect()
        })
    }

    /// Where each id mapped now goes in `new`, the rebuilt graph: the last step
    /// of [`Self::vacuum`], run under the write guard, so the mappings hold
    /// still.
    ///
    /// `rebuilt` gives each snapshot id the slot it held in `old` and the slot
    /// the rebuild gave its vector in `new`. An id still on the slot the
    /// snapshot saw takes its rebuilt slot: `old`'s arena never hands a slot out
    /// twice, and the maintenance lock keeps anything from renumbering, so that
    /// slot still holds the vector the rebuild copied. An id on any other slot
    /// was inserted or upserted since, and its vector is copied from `old` into
    /// `new`. A snapshot id no longer mapped was deleted since: it is left out,
    /// and its node in `new` stays a tombstone, which `vacuum` counts by
    /// setting `next_idx` to the slot count of `new`.
    ///
    /// # Errors
    ///
    /// As [`carry_over`].
    fn reconcile(
        &self,
        old: &HnswInner,
        new: &HnswInner,
        rebuilt: &FxHashMap<u64, (usize, usize)>,
    ) -> Result<Vec<(u64, usize)>, VacuumError> {
        let mut placed = Vec::with_capacity(self.mappings.len());
        let mut written = Vec::new();
        for (id, slot) in self.mappings.iter() {
            match rebuilt.get(&id) {
                Some(&(seen, rebuilt_slot)) if seen == slot => placed.push((id, rebuilt_slot)),
                _ => written.push((id, slot)),
            }
        }
        if !written.is_empty() {
            placed.extend(carry_over(old, new, &written)?);
        }
        Ok(placed)
    }

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

    /// Builds the replacement inner index for [`Self::vacuum`].
    ///
    /// Returns it with the slot each of `live` was given, in order.
    ///
    /// Creates a new graph with the index's own parameters, **preserving the
    /// current backend storage mode** (a RaBitQ index must not silently
    /// downgrade to the Standard f32 backend on vacuum), inserts the active
    /// vectors, and re-installs the trained RaBitQ quantizer when present
    /// (re-encodes the compacted vectors in NodeId order — without this, a
    /// vacuumed RaBitQ index would fall back to f32 search until the next
    /// collection open).
    fn build_vacuum_replacement(
        &self,
        live: &[Live],
    ) -> Result<(HnswInner, Vec<usize>), VacuumError> {
        // The index's own parameters, read from its graph, which is what
        // holds them and what a save persists: `HnswParams::auto` rebuilt
        // every index as if built for its dimension alone, whatever M,
        // ef_construction and alpha it had been given (#2262).
        let (max_connections, ef_construction, alpha, target_mode) = {
            let graph = self.inner.read();
            (
                graph.max_connections(),
                graph.ef_construction(),
                graph.alpha(),
                graph.storage_mode(),
            )
        };
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
            max_connections,
            max_elements: live.len().max(1000),
            ef_construction,
            dimension: self.dimension,
            storage_mode: crate::StorageMode::Full,
            alpha,
            arena_dir: self.arena_dir_for(target_mode),
        })
        .map_err(|e| VacuumError::RebuildFailed(e.to_string()))?;

        let vectors: Vec<&[f32]> = live.iter().map(|live| live.vector.as_slice()).collect();
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
