//! Memory pool for efficient edge allocations.
//!
//! This module provides a simple object pool implementation optimized for
//! graph edge storage with high insert/delete throughput.
//!
//! # EPIC-020 US-003: Memory Pool for Allocations
//!
//! ## Design Decision
//!
//! We use a simple free-list based pool rather than `bumpalo` or `typed-arena`
//! because we need:
//! - Individual deallocation (arenas don't support this)
//! - Thread-safe operations with minimal contention
//! - Predictable memory usage
//!
//! ## Performance Characteristics
//!
//! - Allocation: O(1) amortized (pop from free list or grow)
//! - Deallocation: O(1) (push to free list)
//! - Memory: Pre-allocated chunks reduce fragmentation

use parking_lot::Mutex;
use std::collections::HashSet;
use std::mem::MaybeUninit;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Default chunk size for memory pool (number of items per chunk).
const DEFAULT_CHUNK_SIZE: usize = 1024;

/// A single-threaded memory pool for type `T`.
///
/// Allocates memory in chunks to reduce system allocator overhead
/// and fragmentation.
pub struct MemoryPool<T> {
    chunks: Vec<Box<[MaybeUninit<T>]>>,
    free_indices: Vec<usize>,
    /// Tracks which slots have been initialized via `store()`.
    /// Only initialized slots should be dropped.
    initialized: HashSet<usize>,
    chunk_size: usize,
    total_allocated: usize,
}

impl<T> MemoryPool<T> {
    /// Creates a new memory pool with the specified chunk size.
    #[must_use]
    pub fn new(chunk_size: usize) -> Self {
        Self {
            chunks: Vec::new(),
            free_indices: Vec::new(),
            initialized: HashSet::new(),
            chunk_size: chunk_size.max(1),
            total_allocated: 0,
        }
    }

    /// Creates a new memory pool with default chunk size (1024).
    #[must_use]
    pub fn with_defaults() -> Self {
        Self::new(DEFAULT_CHUNK_SIZE)
    }

    /// Allocates a slot in the pool and returns its index.
    ///
    /// If no free slots are available, grows the pool by one chunk.
    pub fn allocate(&mut self) -> PoolIndex {
        if let Some(index) = self.free_indices.pop() {
            return PoolIndex(index);
        }

        // Need to grow
        self.grow();
        let index = self.total_allocated - 1;
        PoolIndex(index)
    }

    /// Stores a value at the given index.
    ///
    /// # Safety
    ///
    /// The index must have been obtained from `allocate()` and not yet deallocated.
    pub fn store(&mut self, index: PoolIndex, value: T) {
        let (chunk_idx, slot_idx) = self.index_to_chunk_slot(index.0);
        // If already initialized, drop the old value first
        if self.initialized.contains(&index.0) {
            // SAFETY: Slot was previously initialized
            unsafe {
                std::ptr::drop_in_place(self.chunks[chunk_idx][slot_idx].as_mut_ptr());
            }
        }
        // SAFETY: We only store to indices obtained from allocate()
        self.chunks[chunk_idx][slot_idx].write(value);
        self.initialized.insert(index.0);
    }

    /// Gets a reference to the value at the given index.
    ///
    /// Returns `None` if the index is out of bounds or the slot was never initialized
    /// via `store()`.
    #[must_use]
    pub fn get(&self, index: PoolIndex) -> Option<&T> {
        let (chunk_idx, slot_idx) = self.index_to_chunk_slot(index.0);
        // BUG-1 FIX: Check initialized BEFORE assume_init_ref to prevent UB
        if chunk_idx < self.chunks.len() && self.initialized.contains(&index.0) {
            // SAFETY: Slot is in initialized set, meaning store() was called
            Some(unsafe { self.chunks[chunk_idx][slot_idx].assume_init_ref() })
        } else {
            None
        }
    }

    /// Deallocates a slot, making it available for reuse.
    ///
    /// # Safety
    ///
    /// The index must have been obtained from `allocate()` and not already deallocated.
    /// The caller must ensure no references to the value exist.
    pub fn deallocate(&mut self, index: PoolIndex) {
        let (chunk_idx, slot_idx) = self.index_to_chunk_slot(index.0);
        if chunk_idx < self.chunks.len() {
            // Only drop if the slot was initialized
            if self.initialized.remove(&index.0) {
                // SAFETY: Slot was initialized via store()
                unsafe {
                    std::ptr::drop_in_place(self.chunks[chunk_idx][slot_idx].as_mut_ptr());
                }
            }
            self.free_indices.push(index.0);
        }
    }

    /// Returns the number of allocated (in-use) slots.
    #[must_use]
    pub fn allocated_count(&self) -> usize {
        self.total_allocated - self.free_indices.len()
    }

    /// Returns the total capacity of the pool.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.total_allocated
    }

    /// Allocates multiple slots at once for batch operations.
    ///
    /// More efficient than calling `allocate()` repeatedly as it
    /// minimizes grow operations and improves cache locality.
    ///
    /// # Arguments
    ///
    /// * `count` - Number of slots to allocate
    ///
    /// # Returns
    ///
    /// Vector of pool indices for the allocated slots
    pub fn allocate_batch(&mut self, count: usize) -> Vec<PoolIndex> {
        if count == 0 {
            return Vec::new();
        }

        let mut result = Vec::with_capacity(count);

        // First, use available free slots
        let from_free = count.min(self.free_indices.len());
        for _ in 0..from_free {
            if let Some(idx) = self.free_indices.pop() {
                result.push(PoolIndex(idx));
            }
        }

        // If we need more, grow and allocate
        let remaining = count - from_free;
        if remaining > 0 {
            // Calculate how many chunks we need
            let chunks_needed = remaining.div_ceil(self.chunk_size);
            for _ in 0..chunks_needed {
                self.grow_for_batch();
            }

            // Now allocate from free list
            for _ in 0..remaining {
                if let Some(idx) = self.free_indices.pop() {
                    result.push(PoolIndex(idx));
                }
            }
        }

        result
    }

    /// Prefetches a slot for upcoming access (cache warming).
    ///
    /// Call this before accessing a slot to hide memory latency.
    #[inline]
    pub fn prefetch(&self, index: PoolIndex) {
        let (chunk_idx, slot_idx) = self.index_to_chunk_slot(index.0);
        if chunk_idx < self.chunks.len() {
            let ptr = self.chunks[chunk_idx][slot_idx].as_ptr();
            // Use CPU prefetch hint
            #[cfg(target_arch = "x86_64")]
            unsafe {
                std::arch::x86_64::_mm_prefetch(ptr.cast::<i8>(), std::arch::x86_64::_MM_HINT_T0);
            }
            #[cfg(target_arch = "aarch64")]
            {
                crate::simd_neon_prefetch::prefetch_read_l1(ptr.cast::<u8>());
            }
        }
    }

    fn grow(&mut self) {
        let mut chunk: Vec<MaybeUninit<T>> = Vec::with_capacity(self.chunk_size);
        // SAFETY: MaybeUninit doesn't require initialization
        unsafe {
            chunk.set_len(self.chunk_size);
        }
        self.chunks.push(chunk.into_boxed_slice());
        self.total_allocated += self.chunk_size;

        // Add new indices to free list (except the last one which we'll return)
        let start = self.total_allocated - self.chunk_size;
        for i in start..(self.total_allocated - 1) {
            self.free_indices.push(i);
        }
    }

    /// Grows the pool and adds ALL new indices to free list (for batch allocation).
    fn grow_for_batch(&mut self) {
        let mut chunk: Vec<MaybeUninit<T>> = Vec::with_capacity(self.chunk_size);
        // SAFETY: MaybeUninit doesn't require initialization
        unsafe {
            chunk.set_len(self.chunk_size);
        }
        self.chunks.push(chunk.into_boxed_slice());
        self.total_allocated += self.chunk_size;

        // Add ALL new indices to free list (including last one, batch will pop them)
        let start = self.total_allocated - self.chunk_size;
        for i in start..self.total_allocated {
            self.free_indices.push(i);
        }
    }

    #[inline]
    fn index_to_chunk_slot(&self, index: usize) -> (usize, usize) {
        (index / self.chunk_size, index % self.chunk_size)
    }
}

impl<T> Default for MemoryPool<T> {
    fn default() -> Self {
        Self::with_defaults()
    }
}

impl<T> Drop for MemoryPool<T> {
    fn drop(&mut self) {
        // Only drop slots that were actually initialized via store()
        // This fixes UB where allocate() without store() would cause
        // drop_in_place on uninitialized memory
        for &idx in &self.initialized {
            let (chunk_idx, slot_idx) = self.index_to_chunk_slot(idx);
            if chunk_idx < self.chunks.len() {
                // SAFETY: Slot is in initialized set, meaning store() was called
                unsafe {
                    std::ptr::drop_in_place(self.chunks[chunk_idx][slot_idx].as_mut_ptr());
                }
            }
        }
    }
}

/// An index into a memory pool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PoolIndex(usize);

impl PoolIndex {
    /// Returns the raw index value.
    #[must_use]
    pub fn as_usize(self) -> usize {
        self.0
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_pool_allocate_store_get() {
        let mut pool: MemoryPool<u64> = MemoryPool::new(4);

        let idx1 = pool.allocate();
        pool.store(idx1, 42);

        let idx2 = pool.allocate();
        pool.store(idx2, 100);

        assert_eq!(pool.get(idx1), Some(&42));
        assert_eq!(pool.get(idx2), Some(&100));
        assert_eq!(pool.allocated_count(), 2);
    }

    #[test]
    fn test_memory_pool_deallocate_reuse() {
        let mut pool: MemoryPool<u64> = MemoryPool::new(4);

        let idx1 = pool.allocate();
        pool.store(idx1, 42);
        pool.deallocate(idx1);

        // Next allocation should reuse the freed slot
        let idx2 = pool.allocate();
        assert_eq!(idx1.as_usize(), idx2.as_usize());
    }

    #[test]
    fn test_memory_pool_grow() {
        let mut pool: MemoryPool<u64> = MemoryPool::new(2);

        // Allocate more than chunk size
        for i in 0..10 {
            let idx = pool.allocate();
            pool.store(idx, i);
        }

        assert_eq!(pool.allocated_count(), 10);
        assert!(pool.capacity() >= 10);
    }

    #[test]
    fn test_concurrent_pool_basic() {
        let pool: ConcurrentMemoryPool<u64> = ConcurrentMemoryPool::new(2, 4);

        let h1 = pool.allocate();
        pool.store(h1, 42);

        let h2 = pool.allocate();
        pool.store(h2, 100);

        assert_eq!(pool.with_value(h1, |v| *v), Some(42));
        assert_eq!(pool.with_value(h2, |v| *v), Some(100));
    }

    #[test]
    fn test_concurrent_pool_multithread() {
        use std::sync::Arc;
        use std::thread;

        let pool = Arc::new(ConcurrentMemoryPool::<u64>::new(4, 16));
        let mut handles = Vec::new();

        for t in 0..4 {
            let pool_clone = Arc::clone(&pool);
            handles.push(thread::spawn(move || {
                for i in 0..100 {
                    let h = pool_clone.allocate();
                    pool_clone.store(h, (t * 100 + i) as u64);
                }
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(pool.allocated_count(), 400);
    }

    #[test]
    fn test_memory_pool_allocate_batch() {
        let mut pool: MemoryPool<u64> = MemoryPool::new(4);

        // Allocate a batch of 10 slots
        let indices = pool.allocate_batch(10);
        assert_eq!(indices.len(), 10);

        // Store values
        for (i, idx) in indices.iter().enumerate() {
            pool.store(*idx, i as u64 * 10);
        }

        // Verify all values
        for (i, idx) in indices.iter().enumerate() {
            assert_eq!(pool.get(*idx), Some(&(i as u64 * 10)));
        }

        assert_eq!(pool.allocated_count(), 10);
    }

    #[test]
    fn test_memory_pool_allocate_batch_empty() {
        let mut pool: MemoryPool<u64> = MemoryPool::new(4);
        let indices = pool.allocate_batch(0);
        assert!(indices.is_empty());
        assert_eq!(pool.allocated_count(), 0);
    }

    #[test]
    fn test_memory_pool_allocate_batch_with_free_slots() {
        let mut pool: MemoryPool<u64> = MemoryPool::new(4);

        // Allocate and deallocate some slots
        let idx1 = pool.allocate();
        let idx2 = pool.allocate();
        pool.store(idx1, 1);
        pool.store(idx2, 2);
        pool.deallocate(idx1);
        pool.deallocate(idx2);

        // Now batch allocate - should reuse freed slots first
        let indices = pool.allocate_batch(5);
        assert_eq!(indices.len(), 5);
        assert_eq!(pool.allocated_count(), 5);
    }

    #[test]
    fn test_memory_pool_prefetch() {
        let mut pool: MemoryPool<u64> = MemoryPool::new(4);

        let idx = pool.allocate();
        pool.store(idx, 42);

        // Prefetch should not crash
        pool.prefetch(idx);

        // Value should still be accessible
        assert_eq!(pool.get(idx), Some(&42));
    }

    /// Regression test for BUG-1: allocate() without store() must not cause UB in Drop.
    /// Previously, Drop assumed all non-free slots were initialized, which was wrong.
    #[test]
    fn test_allocate_without_store_no_ub() {
        let mut pool: MemoryPool<String> = MemoryPool::new(4);

        // Allocate slots but DON'T call store() - this used to cause UB in Drop
        let _idx1 = pool.allocate();
        let _idx2 = pool.allocate();
        let _idx3 = pool.allocate();

        // Only store to one slot
        let idx4 = pool.allocate();
        pool.store(idx4, "initialized".to_string());

        // Drop should only drop idx4, not the uninitialized slots
        // If this test doesn't crash/ASAN error, the fix works
        drop(pool);
    }

    /// Regression test: deallocate() on uninitialized slot must not crash
    #[test]
    fn test_deallocate_uninitialized_slot() {
        let mut pool: MemoryPool<String> = MemoryPool::new(4);

        let idx = pool.allocate();
        // Don't call store()
        pool.deallocate(idx); // Should not crash

        // Pool should still be usable
        let idx2 = pool.allocate();
        pool.store(idx2, "test".to_string());
        assert_eq!(pool.get(idx2), Some(&"test".to_string()));
    }
}
