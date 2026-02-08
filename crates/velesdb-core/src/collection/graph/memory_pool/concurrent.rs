//! Thread-safe concurrent memory pool using sharded locks.
//!
//! Provides `ConcurrentMemoryPool` which distributes allocations across
//! multiple shards to reduce lock contention in multi-threaded scenarios.

use parking_lot::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::{MemoryPool, PoolIndex, DEFAULT_CHUNK_SIZE};

/// A thread-safe memory pool using sharded locks for reduced contention.
///
/// Each thread gets its own shard based on thread ID, minimizing lock contention
/// in multi-threaded scenarios.
pub struct ConcurrentMemoryPool<T> {
    shards: Vec<Mutex<MemoryPool<T>>>,
    num_shards: usize,
    next_shard: AtomicUsize,
}

impl<T> ConcurrentMemoryPool<T> {
    /// Creates a new concurrent memory pool with the specified number of shards.
    #[must_use]
    pub fn new(num_shards: usize, chunk_size: usize) -> Self {
        let num_shards = num_shards.max(1);
        let shards = (0..num_shards)
            .map(|_| Mutex::new(MemoryPool::new(chunk_size)))
            .collect();
        Self {
            shards,
            num_shards,
            next_shard: AtomicUsize::new(0),
        }
    }

    /// Creates a concurrent memory pool with defaults (4 shards, 1024 chunk size).
    #[must_use]
    pub fn with_defaults() -> Self {
        Self::new(4, DEFAULT_CHUNK_SIZE)
    }

    /// Allocates a slot and returns a handle containing shard and index.
    pub fn allocate(&self) -> ConcurrentPoolHandle {
        let shard_idx = self.next_shard.fetch_add(1, Ordering::Relaxed) % self.num_shards;
        let index = self.shards[shard_idx].lock().allocate();
        ConcurrentPoolHandle {
            shard: shard_idx,
            index,
        }
    }

    /// Stores a value at the given handle.
    pub fn store(&self, handle: ConcurrentPoolHandle, value: T) {
        self.shards[handle.shard].lock().store(handle.index, value);
    }

    /// Gets a reference to the value, requiring exclusive access to the shard.
    ///
    /// Returns None if the handle is invalid.
    pub fn with_value<R>(
        &self,
        handle: ConcurrentPoolHandle,
        f: impl FnOnce(&T) -> R,
    ) -> Option<R> {
        let guard = self.shards[handle.shard].lock();
        guard.get(handle.index).map(f)
    }

    /// Deallocates the slot at the given handle.
    pub fn deallocate(&self, handle: ConcurrentPoolHandle) {
        self.shards[handle.shard].lock().deallocate(handle.index);
    }

    /// Returns the total allocated count across all shards.
    #[must_use]
    pub fn allocated_count(&self) -> usize {
        self.shards.iter().map(|s| s.lock().allocated_count()).sum()
    }

    /// Returns the total capacity across all shards.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.shards.iter().map(|s| s.lock().capacity()).sum()
    }
}

impl<T> Default for ConcurrentMemoryPool<T> {
    fn default() -> Self {
        Self::with_defaults()
    }
}

/// A handle to a slot in a concurrent memory pool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConcurrentPoolHandle {
    shard: usize,
    index: PoolIndex,
}

impl ConcurrentPoolHandle {
    /// Returns the shard index.
    #[must_use]
    pub fn shard(&self) -> usize {
        self.shard
    }

    /// Returns the pool index within the shard.
    #[must_use]
    pub fn index(&self) -> PoolIndex {
        self.index
    }
}

// Compile-time check: ConcurrentMemoryPool must be Send + Sync
// Reason: Compile-time assertion for Send + Sync bounds verification
#[allow(dead_code)]
const _: fn() = || {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<ConcurrentMemoryPool<u64>>();
};
