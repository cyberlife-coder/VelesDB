//! Sharded ID mappings for HNSW index using `DashMap`.
//!
//! This module provides lock-free concurrent bidirectional mapping between
//! external IDs (u64) and internal HNSW indices (usize).
//!
//! # Performance characteristics
//!
//! - **Lock-free reads**: O(1) lookups without blocking
//! - **Sharded writes**: Minimal contention on parallel insertions
//! - **No allocation**: the graph's arena hands out every slot; the mappings
//!   record where each id landed ([`ShardedMappings::assign`], #2246)
//!
//! # EPIC-A.1: Integrated into `HnswIndex`

use std::sync::atomic::{AtomicUsize, Ordering};

use dashmap::DashMap;

use super::native_inner::Placed;

/// Lock-free sharded ID mappings for HNSW index.
///
/// Uses `DashMap` internally for concurrent access without global locks.
/// This enables linear scaling on multi-core systems.
///
/// # Example
///
/// ```rust,ignore
/// // Crate-internal: `ShardedMappings` is not exported.
/// let mappings = ShardedMappings::new();
/// // `placed` is the token a graph placement returns for slot 0: see `Placed`.
/// mappings.assign(42, placed);
/// assert_eq!(mappings.get_idx(42), Some(0));
/// ```
#[derive(Debug)]
pub struct ShardedMappings {
    /// Mapping from external IDs to internal indices (lock-free).
    id_to_idx: DashMap<u64, usize>,
    /// Mapping from internal indices to external IDs (lock-free).
    idx_to_id: DashMap<usize, u64>,
    /// One past the highest slot ever assigned; never decreases until `clear`.
    next_idx: AtomicUsize,
    /// Whether any external id above `u32::MAX` was ever registered.
    ///
    /// Gates the O(N) overflow-id sweep in the bitmap brute-force path: with
    /// no overflow ids (virtually every deployment) that sweep is a pure
    /// full-DashMap scan per filtered query. Sticky by design — removing the
    /// last overflow id keeps the flag set until the index is reloaded, which
    /// costs the sweep again but never correctness.
    has_overflow_ids: std::sync::atomic::AtomicBool,
}

impl Default for ShardedMappings {
    fn default() -> Self {
        Self::new()
    }
}

impl ShardedMappings {
    /// Creates new empty sharded mappings.
    #[must_use]
    pub fn new() -> Self {
        Self {
            id_to_idx: DashMap::new(),
            idx_to_id: DashMap::new(),
            next_idx: AtomicUsize::new(0),
            has_overflow_ids: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Creates mappings with pre-allocated capacity.
    ///
    /// Use this when the expected number of vectors is known upfront.
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            id_to_idx: DashMap::with_capacity(capacity),
            idx_to_id: DashMap::with_capacity(capacity),
            next_idx: AtomicUsize::new(0),
            has_overflow_ids: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Records `id` into the overflow flag (see `has_overflow_ids`).
    #[inline]
    fn note_id(&self, id: u64) {
        if u32::try_from(id).is_err() {
            self.has_overflow_ids
                .store(true, std::sync::atomic::Ordering::Release);
        }
    }

    /// Returns `true` if any id above `u32::MAX` was ever registered.
    #[inline]
    #[must_use]
    pub fn has_overflow_ids(&self) -> bool {
        self.has_overflow_ids
            .load(std::sync::atomic::Ordering::Acquire)
    }

    /// Removes a stale reverse mapping (`idx` -> `id`) without touching the forward mapping.
    ///
    /// Removal is conditional: the entry goes only while it still names
    /// `expected_id`, so retiring a slot can never erase another id's mapping.
    pub(super) fn remove_reverse(&self, idx: usize, expected_id: u64) {
        use dashmap::mapref::entry::Entry;

        let Entry::Occupied(entry) = self.idx_to_id.entry(idx) else {
            return;
        };
        if *entry.get() == expected_id {
            entry.remove();
        }
    }

    /// Maps `id` to the slot `placed` names — the arena slot its vector
    /// already occupies — and returns the slot it held before, when that was
    /// another.
    ///
    /// The one way an insert registers an id: the vector is placed first, by
    /// whoever owns the arena's allocation, and the mapping follows the slot it
    /// got. Nothing predicts a slot, so nothing can hand one slot to two ids
    /// (#2246). The previous slot's reverse entry is retired, which is what
    /// turns the old graph node into a tombstone search filters out.
    ///
    /// `placed` borrows the graph guard its placement ran under, so the slot
    /// cannot be renumbered before it is mapped (see `Placed`). `next_idx` is
    /// raised to at least `slot + 1`, so it stays an upper bound on every slot
    /// in use for its readers (vacuum, persistence).
    pub(crate) fn assign(&self, id: u64, placed: Placed<'_>) -> Option<usize> {
        use dashmap::mapref::entry::Entry;

        let slot = placed.into_slot();
        // Checked before either map changes. The writes then run under the
        // shard lock over the id's entry, in order (forward entry, retire the
        // old reverse entry, claim the new one), so assigns of one id
        // serialize. `remove` drops its two entries one at a time and may
        // interleave, but never with a renumber: its callers hold the graph's
        // read guard. Once writers settle, each id is named by its own slot
        // alone.
        self.refuse_foreign_slot(slot, id);
        self.note_id(id);
        self.next_idx
            .fetch_max(slot.saturating_add(1), Ordering::Relaxed);
        match self.id_to_idx.entry(id) {
            Entry::Occupied(mut entry) => {
                let old = entry.insert(slot);
                if old != slot {
                    self.remove_reverse(old, id);
                }
                self.idx_to_id.insert(slot, id);
                (old != slot).then_some(old)
            }
            Entry::Vacant(entry) => {
                // `insert` hands the shard lock to the reference it returns.
                // The reverse write reads the slot through it, so the lock is
                // held until that write is done, as in the occupied arm.
                let forward = entry.insert(slot);
                self.idx_to_id.insert(*forward, id);
                drop(forward);
                None
            }
        }
    }

    /// Refuses, in a debug build, a slot whose reverse entry names another id.
    ///
    /// The arena hands every slot out once per arena (a vacuum rebuilds the
    /// arena and clears the mappings under the same lock), so such a slot
    /// means a broken caller, not a move. The check only reads, and runs
    /// before either map changes; release builds skip it.
    fn refuse_foreign_slot(&self, slot: usize, id: u64) {
        if cfg!(debug_assertions) {
            if let Some(owner) = self.idx_to_id.get(&slot).map(|owner| *owner) {
                assert_eq!(owner, id, "slot {slot} already belongs to id {owner}");
            }
        }
    }

    /// Removes an ID and returns its internal index if it existed.
    ///
    /// The two writes are not atomic: callers hold the graph's read guard
    /// (`soft_delete` borrows the graph), so no renumber runs between them.
    pub fn remove(&self, id: u64) -> Option<usize> {
        if let Some((_, idx)) = self.id_to_idx.remove(&id) {
            self.idx_to_id.remove(&idx);
            Some(idx)
        } else {
            None
        }
    }

    /// Gets the internal index for an external ID.
    ///
    /// This is a lock-free read operation.
    #[must_use]
    pub fn get_idx(&self, id: u64) -> Option<usize> {
        self.id_to_idx.get(&id).map(|r| *r)
    }

    /// Gets the external ID for an internal index.
    ///
    /// This is a lock-free read operation.
    #[must_use]
    pub fn get_id(&self, idx: usize) -> Option<u64> {
        self.idx_to_id.get(&idx).map(|r| *r)
    }

    /// Returns the number of registered IDs.
    #[must_use]
    pub fn len(&self) -> usize {
        self.id_to_idx.len()
    }

    /// Returns true if no IDs are registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.id_to_idx.is_empty()
    }

    /// Checks if an ID is registered.
    #[must_use]
    pub fn contains(&self, id: u64) -> bool {
        self.id_to_idx.contains_key(&id)
    }

    /// Returns an iterator over all (id, idx) pairs.
    ///
    /// Note: This acquires read locks on shards during iteration.
    pub fn iter(&self) -> impl Iterator<Item = (u64, usize)> + '_ {
        self.id_to_idx.iter().map(|r| (*r.key(), *r.value()))
    }

    /// One past the highest slot ever assigned.
    ///
    /// Never decreases, even after removals, so `next_idx() - len()` counts
    /// the slots below it that no id names any more.
    #[must_use]
    pub fn next_idx(&self) -> usize {
        self.next_idx.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Clears all mappings and resets `next_idx`.
    pub fn clear(&self) {
        self.id_to_idx.clear();
        self.idx_to_id.clear();
        self.next_idx.store(0, std::sync::atomic::Ordering::Relaxed);
    }

    /// Renumbers every internal index through `old_to_new`.
    ///
    /// The graph owns the node numbering, and a BFS locality reorder changes
    /// it. Both directions of this map are keyed by that numbering, so they
    /// have to move with it — `vacuum` renumbers too and rebuilds them from
    /// scratch for the same reason. Skipping it does not fail: every lookup
    /// still resolves, to a different vector than the one asked for, so a
    /// query returns confident, wrong answers (#2112).
    ///
    /// `old_to_new[i]` is the new index of the node that was `i`; a reorder
    /// that succeeds covers every slot of the arena.
    ///
    /// Tombstoned nodes need no special case — deletion removes the mapping
    /// and leaves the node, so they simply have no entry to move.
    ///
    /// Not atomic against concurrent readers. The caller holds the index
    /// write lock across the graph permutation and this call, which is what
    /// keeps a search from observing the half-renumbered state.
    pub fn remap_indices(&self, old_to_new: &[usize]) {
        let renumber = |idx: usize| old_to_new.get(idx).copied().unwrap_or(idx);

        let moved: Vec<(u64, usize)> = self
            .id_to_idx
            .iter()
            .map(|entry| (*entry.key(), renumber(*entry.value())))
            .collect();

        self.id_to_idx.clear();
        self.idx_to_id.clear();
        for (id, idx) in moved {
            self.id_to_idx.insert(id, idx);
            self.idx_to_id.insert(idx, id);
        }
    }

    /// Creates mappings from existing data (for deserialization).
    ///
    /// # Arguments
    ///
    /// * `id_to_idx` - Map from external IDs to internal indices
    /// * `idx_to_id` - Map from internal indices to external IDs
    /// * `next_idx` - One past the highest slot ever assigned
    #[must_use]
    pub fn from_parts(
        id_to_idx: std::collections::HashMap<u64, usize>,
        idx_to_id: std::collections::HashMap<usize, u64>,
        next_idx: usize,
    ) -> Self {
        let sharded_id_to_idx = DashMap::with_capacity(id_to_idx.len());
        let sharded_idx_to_id = DashMap::with_capacity(idx_to_id.len());

        let mut has_overflow = false;
        for (id, idx) in id_to_idx {
            has_overflow |= u32::try_from(id).is_err();
            sharded_id_to_idx.insert(id, idx);
        }
        for (idx, id) in idx_to_id {
            sharded_idx_to_id.insert(idx, id);
        }

        Self {
            id_to_idx: sharded_id_to_idx,
            idx_to_id: sharded_idx_to_id,
            next_idx: AtomicUsize::new(next_idx),
            has_overflow_ids: std::sync::atomic::AtomicBool::new(has_overflow),
        }
    }

    /// Returns cloned data for serialization.
    ///
    /// # Returns
    ///
    /// Tuple of (`id_to_idx`, `idx_to_id`, `next_idx`) for serialization.
    #[must_use]
    pub fn as_parts(
        &self,
    ) -> (
        std::collections::HashMap<u64, usize>,
        std::collections::HashMap<usize, u64>,
        usize,
    ) {
        let id_to_idx: std::collections::HashMap<u64, usize> = self
            .id_to_idx
            .iter()
            .map(|r| (*r.key(), *r.value()))
            .collect();

        let idx_to_id: std::collections::HashMap<usize, u64> = self
            .idx_to_id
            .iter()
            .map(|r| (*r.key(), *r.value()))
            .collect();

        let next_idx = self.next_idx.load(Ordering::SeqCst);

        (id_to_idx, idx_to_id, next_idx)
    }
}
