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
        }
    }

    /// Creates mappings with pre-allocated capacity.
    ///
    /// Use this when the expected number of vectors is known upfront.
    #[must_use]
    #[allow(dead_code)] // API completeness - useful for batch operations
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            id_to_idx: DashMap::with_capacity(capacity),
            idx_to_id: DashMap::with_capacity(capacity),
            next_idx: AtomicUsize::new(0),
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
        // Use entry API for atomic check-and-insert
        use dashmap::mapref::entry::Entry;

        match self.id_to_idx.entry(id) {
            Entry::Occupied(_) => None, // ID already exists
            Entry::Vacant(entry) => {
                // Atomically get next index
                let idx = self.next_idx.fetch_add(1, Ordering::SeqCst);
                entry.insert(idx);
                self.idx_to_id.insert(idx, id);
                Some(idx)
            }
        }
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
    #[allow(dead_code)] // Used by NativeHnswIndex
    pub fn len(&self) -> usize {
        self.id_to_idx.len()
    }

    /// Returns true if no IDs are registered.
    #[must_use]
    #[allow(dead_code)] // API completeness
    pub fn is_empty(&self) -> bool {
        self.id_to_idx.is_empty()
    }

    /// Checks if an ID is registered.
    #[must_use]
    #[allow(dead_code)] // API completeness
    pub fn contains(&self, id: u64) -> bool {
        self.id_to_idx.contains_key(&id)
    }

    /// Returns an iterator over all (id, idx) pairs.
    ///
    /// Note: This acquires read locks on shards during iteration.
    #[allow(dead_code)] // API completeness - useful for debugging
    pub fn iter(&self) -> impl Iterator<Item = (u64, usize)> + '_ {
        self.id_to_idx.iter().map(|r| (*r.key(), *r.value()))
    }

    /// Returns the next available internal index (total inserted count).
    ///
    /// This is a monotonic counter that never decreases, even after removals.
    /// Useful for calculating tombstone count.
    #[must_use]
    #[allow(dead_code)] // API completeness
    pub fn next_idx(&self) -> usize {
        self.next_idx.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Clears all mappings and resets the index counter.
    #[allow(dead_code)] // API completeness
    pub fn clear(&self) {
        self.id_to_idx.clear();
        self.idx_to_id.clear();
        self.next_idx.store(0, std::sync::atomic::Ordering::Relaxed);
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
