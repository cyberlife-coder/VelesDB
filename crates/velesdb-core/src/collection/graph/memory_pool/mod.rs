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

mod concurrent;

#[cfg(test)]
mod tests;

pub use concurrent::{ConcurrentMemoryPool, ConcurrentPoolHandle};

use std::collections::HashSet;
use std::mem::MaybeUninit;

/// Default chunk size for memory pool (number of items per chunk).
pub(crate) const DEFAULT_CHUNK_SIZE: usize = 1024;

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
            // SAFETY: `drop_in_place` requires an initialized value at the pointer.
            // - Condition 1: `initialized.contains(index)` proves `store()` previously wrote here.
            // - Condition 2: Pointer comes from this pool's owned chunk storage.
            // Reason: Replacing existing value must run the old destructor first.
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
            // SAFETY: `assume_init_ref` requires slot initialization.
            // - Condition 1: `initialized` set confirms `store()` initialized this slot.
            // - Condition 2: Index bounds are checked by `chunk_idx < self.chunks.len()`.
            // Reason: Return borrowed view without copying pooled object.
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
                // SAFETY: `drop_in_place` requires an initialized value.
                // - Condition 1: `initialized.remove(index)` confirms previous initialization.
                // - Condition 2: Pointer refers to this pool's owned chunk memory.
                // Reason: Deallocation must run destructor before slot reuse.
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
            // SAFETY: `_mm_prefetch` accepts any address as a cache hint.
            // - Condition 1: `ptr` is derived from a valid in-bounds slot pointer.
            // - Condition 2: Prefetch does not dereference or mutate memory.
            // Reason: Cache warming reduces traversal latency.
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
        // SAFETY: `set_len` is valid because elements are `MaybeUninit<T>`.
        // - Condition 1: Capacity was allocated for `chunk_size` elements.
        // - Condition 2: `MaybeUninit<T>` has no initialization requirement.
        // Reason: Preallocate pool slots without constructing `T` values.
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
        // SAFETY: `set_len` is valid because elements are `MaybeUninit<T>`.
        // - Condition 1: Capacity was allocated for `chunk_size` elements.
        // - Condition 2: `MaybeUninit<T>` has no initialization requirement.
        // Reason: Batch growth reserves uninitialized slots for later writes.
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
                // SAFETY: `drop_in_place` requires initialized memory.
                // - Condition 1: `initialized` set only contains slots written via `store()`.
                // - Condition 2: Index translation resolves into this pool's owned chunk.
                // Reason: Drop must clean initialized elements and skip uninitialized slots.
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
