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

/// Each id whose vector `vacuum` has put in the new graph, with the slot it held
/// in the old graph when that vector was read there and the slot the vector
/// got in the new one.
type Carried = FxHashMap<u64, (usize, usize)>;

/// Most catch-up rounds [`HnswIndex::vacuum`] runs before it takes the write
/// guard. Each round copies the writes made during the one before, so under a
/// steady write rate the rounds shrink as long as copying is faster than
/// writing; the bound ends a vacuum that writes outpace, which then copies
/// what is left under the write guard (see [`CATCH_UP_REMAINDER`]).
const MAX_CATCH_UP_ROUNDS: usize = 4;

/// Writes left at or under which `vacuum` stops catching up and takes the
/// write guard, where it copies them one by one while every search waits.
///
/// This is when the catch-up stops trying, not a bound on what it leaves.
/// [`HnswIndex::reconcile`] copies every id mapped but not carried when the guard
/// is granted, and a write in flight joins that set after this check: a batch
/// assigns its ids between the last round and the guard, so a round that saw
/// nothing left can still be followed by a copy of the whole batch. Bounding
/// it — adaptive rounds, or a sealed watermark as velesdb-memory's online
/// migration uses — is tracked in #2335.
const CATCH_UP_REMAINDER: usize = 64;

/// The slot `id` got in the new graph, if `carried` holds its vector as it is
/// now: the id still sits on the old slot its vector was read from. The old
/// graph's arena never hands a slot out twice, and the maintenance lock keeps
/// anything from renumbering, so that slot still holds that vector.
fn carried_slot(carried: &Carried, id: u64, slot: usize) -> Option<usize> {
    carried
        .get(&id)
        .and_then(|&(seen, new_slot)| (seen == slot).then_some(new_slot))
}

/// The vector at each slot of `written` in `old`, in order.
///
/// # Errors
///
/// [`VacuumError::RebuildFailed`] if a slot holds no vector in `old`.
fn copy_vectors(old: &HnswInner, written: &[(u64, usize)]) -> Result<Vec<Vec<f32>>, VacuumError> {
    let vectors: Option<Vec<Vec<f32>>> = old.with_contiguous_vectors(|arena| {
        written
            .iter()
            .map(|&(_, slot)| arena.get(slot).map(<[f32]>::to_vec))
            .collect()
    });
    vectors.ok_or_else(|| {
        VacuumError::RebuildFailed("a mapped slot holds no vector in the old graph".into())
    })
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
    ///   searches and writes carry on against the old graph. The vectors of
    ///   the ids inserted or upserted since the snapshot are then copied from
    ///   the old graph into the new one, still without the write lock, in a
    ///   bounded number of rounds, each copying the writes made during the one
    ///   before. The swap re-maps the ids mapped at that moment, not the
    ///   snapshot's: it copies, one at a time, every id mapped but not yet
    ///   carried when the guard is granted, and an id deleted since stays
    ///   deleted. The rounds stop once few writes are left, or after a fixed
    ///   number of them; that is when the catch-up stops trying, and bounds
    ///   nothing. A write in flight, a whole batch, maps its ids after the
    ///   last round looked and before the guard is granted, so the swap can
    ///   copy them all (#2335). One write lock covers
    ///   those last copies, the re-map of every live id and dropping the old
    ///   graph, so a search never sees a half-built mapping, and waits for all
    ///   three.
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
        // Nothing to reclaim only when the graph holds no slot at all. With no
        // live id but dead slots, the rebuild still runs, into an empty graph:
        // returning here left every dead slot in place, and the tombstone
        // count asking for a vacuum that reclaimed none (#2262).
        if live.is_empty() && self.graph_vector_count() == 0 {
            return Ok(0);
        }

        // 2-4. Rebuild a fresh inner index from the snapshot, preserving the
        // backend storage mode and trained quantizer. No graph guard is held:
        // writers carry on against the old graph meanwhile.
        let (new_inner, slots) = self.build_vacuum_replacement(&live)?;

        // 5. Copy the writes made during the rebuild into the new graph, still
        // without the write guard, until few are left (#2262).
        let carried = self.catch_up(&new_inner, live, slots)?;

        // 6-7. Swap in the new graph and rebuild the mappings under one write
        // lock: until the rebuild they name the old graph's slots, and a
        // search in between would resolve ids against the wrong vectors
        // (#2246). They are rebuilt from what is mapped now, not from the
        // snapshot, or every write made during the rebuild would be lost (#2262).
        let placed = {
            let mut inner_guard = self.inner.write();
            let placed = self.reconcile(&inner_guard, &new_inner, &carried)?;
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

    /// Step 5 of [`Self::vacuum`]: where each id's vector is in `new`, which
    /// the rebuild filled with `live`, giving each the slot of `slots` at the
    /// same position.
    ///
    /// Starting from that, it copies into `new` the vectors of the ids written
    /// since they were last read, and records where each went, round after
    /// round, until at most [`CATCH_UP_REMAINDER`] are left or
    /// [`MAX_CATCH_UP_ROUNDS`] have run.
    ///
    /// No write guard is held, so this may run on rayon: `new` is this
    /// vacuum's own graph, and each round holds a read guard only while it
    /// lists the written ids and copies their vectors out, never while it
    /// inserts them. Under the write guard, a batch insert waited on rayon
    /// workers that were parked on that very guard by batch searches, and the
    /// vacuum hung for ever. What this leaves, [`Self::reconcile`] copies one
    /// by one under the write guard.
    ///
    /// # Errors
    ///
    /// [`VacuumError::RebuildFailed`] if the rebuild did not give each of
    /// `live` one slot, if a written slot holds no vector in the old graph, or
    /// if `new` refuses one.
    fn catch_up(
        &self,
        new: &HnswInner,
        live: Vec<Live>,
        slots: Vec<usize>,
    ) -> Result<Carried, VacuumError> {
        let slots = one_slot_each(slots, live.len())?;
        let mut carried: Carried = live
            .into_iter()
            .zip(slots)
            .map(|(live, slot)| (live.id, (live.slot, slot)))
            .collect();
        for _ in 0..MAX_CATCH_UP_ROUNDS {
            let old = self.inner.read();
            let written = self.written_since(&carried);
            if written.len() <= CATCH_UP_REMAINDER {
                return Ok(carried);
            }
            let vectors = copy_vectors(&old, &written)?;
            drop(old);
            let refs: Vec<&[f32]> = vectors.iter().map(Vec::as_slice).collect();
            let slots = new
                .parallel_insert(&refs)
                .map_err(|e| VacuumError::RebuildFailed(e.to_string()))?;
            let slots = one_slot_each(slots, refs.len())?;
            carried.extend(
                written
                    .into_iter()
                    .zip(slots)
                    .map(|((id, seen), slot)| (id, (seen, slot))),
            );
        }
        Ok(carried)
    }

    /// Each id mapped now whose vector `carried` does not hold as it is now,
    /// with its slot: inserted since, or upserted onto another slot.
    fn written_since(&self, carried: &Carried) -> Vec<(u64, usize)> {
        self.mappings
            .iter()
            .filter(|&(id, slot)| carried_slot(carried, id, slot).is_none())
            .collect()
    }

    /// Where each id mapped now goes in `new`, the rebuilt graph: the last step
    /// of [`Self::vacuum`], run under the write guard, so the mappings hold
    /// still.
    ///
    /// `carried` gives each id whose vector is already in `new` the slot it
    /// held in `old` when that vector was read, and the slot it got in `new`.
    /// An id still on that slot takes its slot in `new` (see [`carried_slot`]).
    /// An id on any other slot was written since, and its vector is copied
    /// from `old` into `new` here, one insert at a time: rayon must not run
    /// under the write guard (see [`Self::catch_up`]). An id of `carried` no
    /// longer mapped was deleted since: it is left out, and its node in `new`
    /// stays a tombstone, which `vacuum` counts by setting `next_idx` to the
    /// slot count of `new`.
    ///
    /// # Errors
    ///
    /// As [`Self::catch_up`].
    fn reconcile(
        &self,
        old: &HnswInner,
        new: &HnswInner,
        carried: &Carried,
    ) -> Result<Vec<(u64, usize)>, VacuumError> {
        let mut placed = Vec::with_capacity(self.mappings.len());
        let mut written = Vec::new();
        for (id, slot) in self.mappings.iter() {
            match carried_slot(carried, id, slot) {
                Some(new_slot) => placed.push((id, new_slot)),
                None => written.push((id, slot)),
            }
        }
        let vectors = copy_vectors(old, &written)?;
        for ((id, _), vector) in written.into_iter().zip(&vectors) {
            let slot = new
                .insert(vector)
                .map_err(|e| VacuumError::RebuildFailed(e.to_string()))?;
            placed.push((id, slot));
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
