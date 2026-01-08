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

// ============================================================================
// TDD TESTS - Written BEFORE implementation (following /rust-feature workflow)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    // -------------------------------------------------------------------------
    // Basic functionality tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_sharded_mappings_new_is_empty() {
        // Arrange & Act
        let mappings = ShardedMappings::new();

        // Assert
        assert!(mappings.is_empty());
        assert_eq!(mappings.len(), 0);
    }

    #[test]
    fn test_sharded_mappings_register_returns_index() {
        // Arrange
        let mappings = ShardedMappings::new();

        // Act
        let idx = mappings.register(42);

        // Assert
        assert_eq!(idx, Some(0));
        assert_eq!(mappings.len(), 1);
    }

    #[test]
    fn test_sharded_mappings_register_increments_index() {
        // Arrange
        let mappings = ShardedMappings::new();

        // Act & Assert
        assert_eq!(mappings.register(1), Some(0));
        assert_eq!(mappings.register(2), Some(1));
        assert_eq!(mappings.register(3), Some(2));
        assert_eq!(mappings.len(), 3);
    }

    #[test]
    fn test_sharded_mappings_register_duplicate_returns_none() {
        // Arrange
        let mappings = ShardedMappings::new();
        mappings.register(42);

        // Act
        let result = mappings.register(42);

        // Assert
        assert_eq!(result, None);
        assert_eq!(mappings.len(), 1);
    }

    #[test]
    fn test_sharded_mappings_get_idx() {
        // Arrange
        let mappings = ShardedMappings::new();
        mappings.register(42);

        // Act & Assert
        assert_eq!(mappings.get_idx(42), Some(0));
        assert_eq!(mappings.get_idx(999), None);
    }

    #[test]
    fn test_sharded_mappings_get_id() {
        // Arrange
        let mappings = ShardedMappings::new();
        mappings.register(42);

        // Act & Assert
        assert_eq!(mappings.get_id(0), Some(42));
        assert_eq!(mappings.get_id(999), None);
    }

    #[test]
    fn test_sharded_mappings_remove() {
        // Arrange
        let mappings = ShardedMappings::new();
        mappings.register(42);

        // Act
        let result = mappings.remove(42);

        // Assert
        assert_eq!(result, Some(0));
        assert!(mappings.is_empty());
        assert_eq!(mappings.get_idx(42), None);
        assert_eq!(mappings.get_id(0), None);
    }

    #[test]
    fn test_sharded_mappings_remove_nonexistent() {
        // Arrange
        let mappings = ShardedMappings::new();

        // Act & Assert
        assert_eq!(mappings.remove(999), None);
    }

    #[test]
    fn test_sharded_mappings_contains() {
        // Arrange
        let mappings = ShardedMappings::new();
        mappings.register(42);

        // Act & Assert
        assert!(mappings.contains(42));
        assert!(!mappings.contains(999));
    }

    #[test]
    fn test_sharded_mappings_with_capacity() {
        // Arrange & Act
        let mappings = ShardedMappings::with_capacity(1000);

        // Assert - should work like normal but with pre-allocated capacity
        assert!(mappings.is_empty());
        assert_eq!(mappings.register(1), Some(0));
    }

    #[test]
    fn test_sharded_mappings_register_batch() {
        // Arrange
        let mappings = ShardedMappings::new();
        let ids = vec![10, 20, 30, 40, 50];

        // Act
        let results = mappings.register_batch(&ids);

        // Assert
        assert_eq!(results.len(), 5);
        assert_eq!(mappings.len(), 5);
        for (id, idx) in results {
            assert_eq!(mappings.get_idx(id), Some(idx));
        }
    }

    #[test]
    fn test_sharded_mappings_register_batch_with_duplicates() {
        // Arrange
        let mappings = ShardedMappings::new();
        mappings.register(20); // Pre-register one ID

        let ids = vec![10, 20, 30]; // 20 is duplicate

        // Act
        let results = mappings.register_batch(&ids);

        // Assert - only 2 new registrations (10 and 30)
        assert_eq!(results.len(), 2);
        assert_eq!(mappings.len(), 3);
    }

    #[test]
    fn test_sharded_mappings_iter() {
        // Arrange
        let mappings = ShardedMappings::new();
        mappings.register(10);
        mappings.register(20);
        mappings.register(30);

        // Act
        let items: Vec<(u64, usize)> = mappings.iter().collect();

        // Assert
        assert_eq!(items.len(), 3);
    }

    // -------------------------------------------------------------------------
    // Concurrency tests - Critical for EPIC-A validation
    // -------------------------------------------------------------------------

    #[test]
    fn test_sharded_mappings_concurrent_register() {
        // Arrange
        let mappings = Arc::new(ShardedMappings::new());
        let num_threads = 8;
        let ids_per_thread = 1000;

        // Act - spawn threads that register unique IDs
        let handles: Vec<_> = (0..num_threads)
            .map(|t| {
                let m = Arc::clone(&mappings);
                thread::spawn(move || {
                    let start = t * ids_per_thread;
                    let end = start + ids_per_thread;
                    let mut registered = 0;
                    for id in start..end {
                        if m.register(id as u64).is_some() {
                            registered += 1;
                        }
                    }
                    registered
                })
            })
            .collect();

        let total: usize = handles.into_iter().map(|h| h.join().unwrap()).sum();

        // Assert - all unique IDs should be registered exactly once
        assert_eq!(total, num_threads * ids_per_thread);
        assert_eq!(mappings.len(), num_threads * ids_per_thread);
    }

    #[test]
    fn test_sharded_mappings_concurrent_register_same_ids() {
        // Arrange - multiple threads trying to register SAME IDs
        let mappings = Arc::new(ShardedMappings::new());
        let num_threads = 16;
        let num_ids = 100; // Each thread tries to register same 100 IDs

        // Act
        let handles: Vec<_> = (0..num_threads)
            .map(|_| {
                let m = Arc::clone(&mappings);
                thread::spawn(move || {
                    let mut registered = 0;
                    for id in 0..num_ids {
                        if m.register(id as u64).is_some() {
                            registered += 1;
                        }
                    }
                    registered
                })
            })
            .collect();

        let total: usize = handles.into_iter().map(|h| h.join().unwrap()).sum();

        // Assert - each ID should be registered exactly once across all threads
        assert_eq!(total, num_ids);
        assert_eq!(mappings.len(), num_ids);
    }

    #[test]
    fn test_sharded_mappings_concurrent_read_write() {
        // Arrange - readers and writers operating simultaneously
        let mappings = Arc::new(ShardedMappings::new());

        // Pre-populate some data
        for i in 0..1000 {
            mappings.register(i);
        }

        let num_readers = 4;
        let num_writers = 4;

        // Act - readers and writers in parallel
        let mut handles = vec![];

        // Readers
        for _ in 0..num_readers {
            let m = Arc::clone(&mappings);
            handles.push(thread::spawn(move || {
                for _ in 0..10000 {
                    let _ = m.get_idx(500);
                    let _ = m.get_id(500);
                    let _ = m.contains(500);
                }
            }));
        }

        // Writers
        for t in 0..num_writers {
            let m = Arc::clone(&mappings);
            handles.push(thread::spawn(move || {
                let start = 1000 + t * 100;
                for i in start..(start + 100) {
                    m.register(i as u64);
                }
            }));
        }

        // Assert - no deadlocks, no panics
        for h in handles {
            h.join().expect("Thread should not panic");
        }

        // Final count should be 1000 + (4 writers * 100 IDs each)
        assert_eq!(mappings.len(), 1000 + num_writers * 100);
    }

    #[test]
    fn test_sharded_mappings_no_data_race() {
        // This test validates that concurrent operations don't corrupt data
        let mappings = Arc::new(ShardedMappings::new());
        let num_threads = 8;
        let ops_per_thread = 1000;

        let handles: Vec<_> = (0..num_threads)
            .map(|t| {
                let m = Arc::clone(&mappings);
                thread::spawn(move || {
                    for i in 0..ops_per_thread {
                        #[allow(clippy::cast_sign_loss)]
                        let id = (t * ops_per_thread + i) as u64;

                        // Register
                        if let Some(idx) = m.register(id) {
                            // Verify immediately
                            assert_eq!(m.get_idx(id), Some(idx));
                            assert_eq!(m.get_id(idx), Some(id));
                        }
                    }
                })
            })
            .collect();

        for h in handles {
            h.join().expect("No data race");
        }

        // Verify all mappings are consistent
        for entry in mappings.iter() {
            let (id, idx) = entry;
            assert_eq!(mappings.get_idx(id), Some(idx));
            assert_eq!(mappings.get_id(idx), Some(id));
        }
    }

    // -------------------------------------------------------------------------
    // Serialization tests (TDD for HnswIndex migration)
    // -------------------------------------------------------------------------

    #[test]
    fn test_sharded_mappings_as_parts_empty() {
        // Arrange
        let mappings = ShardedMappings::new();

        // Act
        let (id_to_idx, idx_to_id, next_idx) = mappings.as_parts();

        // Assert
        assert!(id_to_idx.is_empty());
        assert!(idx_to_id.is_empty());
        assert_eq!(next_idx, 0);
    }

    #[test]
    fn test_sharded_mappings_as_parts_with_data() {
        // Arrange
        let mappings = ShardedMappings::new();
        mappings.register(100);
        mappings.register(200);
        mappings.register(300);

        // Act
        let (id_to_idx, idx_to_id, next_idx) = mappings.as_parts();

        // Assert
        assert_eq!(id_to_idx.len(), 3);
        assert_eq!(idx_to_id.len(), 3);
        assert_eq!(next_idx, 3);
        assert_eq!(id_to_idx.get(&100), Some(&0));
        assert_eq!(id_to_idx.get(&200), Some(&1));
        assert_eq!(id_to_idx.get(&300), Some(&2));
    }

    #[test]
    fn test_sharded_mappings_from_parts_roundtrip() {
        // Arrange - Create original mappings
        let original = ShardedMappings::new();
        original.register(42);
        original.register(100);
        original.register(999);

        // Act - Serialize and deserialize
        let (id_to_idx, idx_to_id, next_idx) = original.as_parts();
        let restored = ShardedMappings::from_parts(id_to_idx, idx_to_id, next_idx);

        // Assert - Data preserved
        assert_eq!(restored.len(), 3);
        assert_eq!(restored.get_idx(42), Some(0));
        assert_eq!(restored.get_idx(100), Some(1));
        assert_eq!(restored.get_idx(999), Some(2));
        assert_eq!(restored.get_id(0), Some(42));
        assert_eq!(restored.get_id(1), Some(100));
        assert_eq!(restored.get_id(2), Some(999));
    }

    #[test]
    fn test_sharded_mappings_from_parts_preserves_next_idx() {
        // Arrange
        let original = ShardedMappings::new();
        original.register(1);
        original.register(2);

        // Act
        let (id_to_idx, idx_to_id, next_idx) = original.as_parts();
        let restored = ShardedMappings::from_parts(id_to_idx, idx_to_id, next_idx);

        // Register new ID - should get next_idx = 2
        let new_idx = restored.register(3);

        // Assert
        assert_eq!(new_idx, Some(2));
    }
}
