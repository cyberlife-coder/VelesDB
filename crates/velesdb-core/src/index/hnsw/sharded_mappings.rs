//! Sharded ID mappings for HNSW index using `DashMap`.
//!
//! This module provides lock-free concurrent bidirectional mapping between
//! external IDs (u64) and internal HNSW indices (usize).
//!
//! # Performance characteristics
//!
//! - **Lock-free reads**: O(1) lookups without blocking
//! - **Sharded writes**: Minimal contention on parallel insertions
//! - **Atomic counter**: Lock-free index allocation
//!
//! # EPIC-A.1: Integrated into `HnswIndex`

use dashmap::DashMap;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Lock-free sharded ID mappings for HNSW index.
///
/// Uses `DashMap` internally for concurrent access without global locks.
/// This enables linear scaling on multi-core systems.
///
/// # Tombstone slots
///
/// When `batch_insert_fast_path` detects a concurrent race on a pre-reserved
/// index range, the colliding slot becomes an orphaned "tombstone" that is
/// never reused. These are harmless (the monotonic counter never wraps) but
/// can be monitored via [`Self::tombstone_count`].
///
/// # Example
///
/// ```rust,ignore
/// use velesdb_core::index::hnsw::ShardedMappings;
///
/// let mappings = ShardedMappings::new();
/// let idx = mappings.register(42).unwrap();
/// assert_eq!(mappings.get_idx(42), Some(0));
/// ```
#[derive(Debug)]
pub struct ShardedMappings {
    /// Mapping from external IDs to internal indices (lock-free).
    id_to_idx: DashMap<u64, usize>,
    /// Mapping from internal indices to external IDs (lock-free).
    idx_to_id: DashMap<usize, u64>,
    /// Next available internal index (atomic for lock-free increment).
    next_idx: AtomicUsize,
    /// Number of orphaned index slots created by race conditions in
    /// `batch_insert_fast_path`. Monotonically increasing.
    tombstone_slots: AtomicUsize,
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
            tombstone_slots: AtomicUsize::new(0),
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
            tombstone_slots: AtomicUsize::new(0),
        }
    }

    /// Registers an ID and returns its internal index.
    ///
    /// Returns `None` if the ID already exists (no duplicate insertions).
    ///
    /// # Thread Safety
    ///
    /// This operation is atomic - concurrent calls with the same ID will
    /// return `Some` for exactly one caller and `None` for others.
    pub fn register(&self, id: u64) -> Option<usize> {
        use dashmap::mapref::entry::Entry;

        match self.id_to_idx.entry(id) {
            Entry::Occupied(_) => None,
            Entry::Vacant(entry) => Some(self.allocate_and_map(entry, id)),
        }
    }

    /// Registers an ID, replacing the existing mapping if present.
    ///
    /// Returns `(new_internal_idx, Option<old_internal_idx>)`:
    /// - If the ID is new: `(idx, None)`
    /// - If the ID existed: `(new_idx, Some(old_idx))`
    ///
    /// The old internal index is removed from the reverse mapping so that
    /// stale HNSW graph nodes are filtered out during search.
    ///
    /// # Thread Safety
    ///
    /// Uses `DashMap::entry()` for atomic check-and-replace. Concurrent
    /// calls with the same ID are serialised by the entry lock.
    pub fn register_or_replace(&self, id: u64) -> (usize, Option<usize>) {
        use dashmap::mapref::entry::Entry;

        match self.id_to_idx.entry(id) {
            Entry::Occupied(mut entry) => {
                let old_idx = *entry.get();
                let new_idx = self.next_idx.fetch_add(1, Ordering::Relaxed);
                entry.insert(new_idx);
                self.idx_to_id.remove(&old_idx);
                self.idx_to_id.insert(new_idx, id);
                (new_idx, Some(old_idx))
            }
            Entry::Vacant(entry) => (self.allocate_and_map(entry, id), None),
        }
    }

    /// Allocates a new internal index and inserts bidirectional mappings.
    ///
    /// Shared by `register` and `register_or_replace` for the new-ID path.
    fn allocate_and_map(
        &self,
        entry: dashmap::mapref::entry::VacantEntry<'_, u64, usize>,
        id: u64,
    ) -> usize {
        let idx = self.next_idx.fetch_add(1, Ordering::Relaxed);
        entry.insert(idx);
        self.idx_to_id.insert(idx, id);
        idx
    }

    /// Batch version of `register_or_replace` with a fast path for pure inserts.
    ///
    /// **Fast path** (all IDs are new — common for batch-insert workloads):
    /// reserves a contiguous index range with a single `fetch_add(N)` instead
    /// of N individual atomic increments. Each ID is still verified via
    /// `DashMap::entry()` to handle concurrent races; if a race is detected
    /// the method falls back to per-ID allocation for that entry.
    ///
    /// **Slow path** (at least one ID already exists): processes each ID
    /// individually with one `entry()` call per ID, replacing stale mappings.
    pub fn register_or_replace_batch(&self, ids: &[u64]) -> Vec<(usize, Option<usize>)> {
        if ids.is_empty() {
            return Vec::new();
        }

        let all_vacant = ids.iter().all(|id| !self.id_to_idx.contains_key(id));

        if all_vacant {
            self.batch_insert_fast_path(ids)
        } else {
            self.batch_replace_slow_path(ids)
        }
    }

    /// Fast path: all IDs are new. Reserves `[start, start+N)` with one atomic op.
    ///
    /// If a concurrent insert races between the vacancy check and `entry()`,
    /// the affected ID falls back to individual `fetch_add(1)` allocation.
    fn batch_insert_fast_path(&self, ids: &[u64]) -> Vec<(usize, Option<usize>)> {
        use dashmap::mapref::entry::Entry;

        let n = ids.len();
        let start = self.next_idx.fetch_add(n, Ordering::Relaxed);

        let mut results = Vec::with_capacity(n);
        for (i, &id) in ids.iter().enumerate() {
            let result = match self.id_to_idx.entry(id) {
                Entry::Vacant(entry) => {
                    let idx = start + i;
                    entry.insert(idx);
                    self.idx_to_id.insert(idx, id);
                    (idx, None)
                }
                Entry::Occupied(mut entry) => {
                    // Race: another thread inserted this ID after our vacancy check.
                    // Use individual allocation (the range slot `start+i` becomes a
                    // tombstone — harmless, as next_idx is monotonic and never reused).
                    self.tombstone_slots.fetch_add(1, Ordering::Relaxed);
                    let old_idx = *entry.get();
                    let new_idx = self.next_idx.fetch_add(1, Ordering::Relaxed);
                    entry.insert(new_idx);
                    self.idx_to_id.remove(&old_idx);
                    self.idx_to_id.insert(new_idx, id);
                    (new_idx, Some(old_idx))
                }
            };
            results.push(result);
        }
        results
    }

    /// Slow path: at least one ID exists. Processes each ID individually.
    fn batch_replace_slow_path(&self, ids: &[u64]) -> Vec<(usize, Option<usize>)> {
        use dashmap::mapref::entry::Entry;

        let mut results = Vec::with_capacity(ids.len());
        for &id in ids {
            let result = match self.id_to_idx.entry(id) {
                Entry::Vacant(entry) => (self.allocate_and_map(entry, id), None),
                Entry::Occupied(mut entry) => {
                    let old_idx = *entry.get();
                    let new_idx = self.next_idx.fetch_add(1, Ordering::Relaxed);
                    entry.insert(new_idx);
                    self.idx_to_id.remove(&old_idx);
                    self.idx_to_id.insert(new_idx, id);
                    (new_idx, Some(old_idx))
                }
            };
            results.push(result);
        }
        results
    }

    /// Registers multiple IDs in a batch, returning their indices.
    ///
    /// # Returns
    ///
    /// Vector of (id, idx) pairs for successfully registered IDs.
    /// IDs that already exist are skipped.
    #[allow(dead_code)] // API completeness - useful for batch operations
    pub fn register_batch(&self, ids: &[u64]) -> Vec<(u64, usize)> {
        let mut results = Vec::with_capacity(ids.len());

        for &id in ids {
            if let Some(idx) = self.register(id) {
                results.push((id, idx));
            }
        }

        results
    }

    /// Restores a specific mapping (`id` -> `idx`) without allocating a new index.
    ///
    /// Used for rollback after a failed graph insertion: re-links the external
    /// ID to a previously-allocated internal index that was removed by
    /// `register_or_replace` or `remove`.
    ///
    /// # Correctness
    ///
    /// The caller must ensure `idx` was previously returned by `register` or
    /// `register_or_replace` for this `id`. Passing an arbitrary `idx` will
    /// corrupt the bidirectional mapping.
    pub fn restore(&self, id: u64, idx: usize) {
        self.id_to_idx.insert(id, idx);
        self.idx_to_id.insert(idx, id);
    }

    /// Removes a stale reverse mapping (`idx` -> `id`) without touching the forward mapping.
    ///
    /// Used when `insert_and_correct_mapping` detects a concurrent race: the
    /// forward mapping `id -> idx` was already corrected by `restore()`, but the
    /// old `idx_to_id[old_idx]` entry is still dangling.
    pub fn remove_reverse(&self, idx: usize) {
        self.idx_to_id.remove(&idx);
    }

    /// Removes an ID and returns its internal index if it existed.
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

    /// Returns the next available internal index (total inserted count).
    ///
    /// This is a monotonic counter that never decreases, even after removals.
    #[must_use]
    pub fn next_idx(&self) -> usize {
        self.next_idx.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Returns the number of orphaned index slots created by race conditions
    /// in [`Self::register_or_replace_batch`]'s fast path.
    ///
    /// These slots are harmless (monotonic counter, never reused) but this
    /// metric is useful for monitoring contention in concurrent batch inserts.
    #[must_use]
    #[allow(dead_code)] // API completeness — used in tests and available for monitoring
    pub fn tombstone_count(&self) -> usize {
        self.tombstone_slots.load(Ordering::Relaxed)
    }

    /// Clears all mappings and resets the index and tombstone counters.
    pub fn clear(&self) {
        self.id_to_idx.clear();
        self.idx_to_id.clear();
        self.next_idx.store(0, std::sync::atomic::Ordering::Relaxed);
        self.tombstone_slots.store(0, Ordering::Relaxed);
    }

    /// Creates mappings from existing data (for deserialization).
    ///
    /// # Arguments
    ///
    /// * `id_to_idx` - Map from external IDs to internal indices
    /// * `idx_to_id` - Map from internal indices to external IDs
    /// * `next_idx` - Next available internal index
    #[must_use]
    pub fn from_parts(
        id_to_idx: std::collections::HashMap<u64, usize>,
        idx_to_id: std::collections::HashMap<usize, u64>,
        next_idx: usize,
    ) -> Self {
        let sharded_id_to_idx = DashMap::with_capacity(id_to_idx.len());
        let sharded_idx_to_id = DashMap::with_capacity(idx_to_id.len());

        for (id, idx) in id_to_idx {
            sharded_id_to_idx.insert(id, idx);
        }
        for (idx, id) in idx_to_id {
            sharded_idx_to_id.insert(idx, id);
        }

        Self {
            id_to_idx: sharded_id_to_idx,
            idx_to_id: sharded_idx_to_id,
            next_idx: AtomicUsize::new(next_idx),
            tombstone_slots: AtomicUsize::new(0),
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
