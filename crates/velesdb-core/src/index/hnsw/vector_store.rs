//! Contiguous vector storage for improved cache locality.
//!
//! This module provides a memory-efficient vector storage that keeps all vectors
//! in a single contiguous memory block, improving cache hit rates during search.
//!
//! # Performance Benefits
//!
//! | Storage Type | Cache Locality | Memory Overhead |
//! |--------------|----------------|-----------------|
//! | FxHashMap    | Poor (scattered)| ~40 bytes/entry |
//! | VectorStore  | Excellent      | ~8 bytes/entry  |

// Allow dead_code - VectorStore is a new optimization module that will be
// integrated into HnswIndex in a future update. Tests verify correctness.
#![allow(dead_code)]

use parking_lot::RwLock;

/// Contiguous vector storage with O(1) access.
///
/// Vectors are stored in a single `Vec<f32>` buffer, with each vector
/// occupying `dimension` consecutive elements. This provides:
/// - Better cache locality during sequential access
/// - Reduced memory fragmentation
/// - Lower memory overhead per vector
///
/// # Memory Layout
///
/// ```text
/// Buffer: [v0_d0, v0_d1, ..., v0_dn, v1_d0, v1_d1, ..., v1_dn, ...]
/// Index:  |<---- vector 0 ---->|    |<---- vector 1 ---->|
/// ```
pub struct VectorStore {
    /// Contiguous buffer holding all vectors
    buffer: RwLock<Vec<f32>>,
    /// Vector dimension
    dimension: usize,
    /// Number of vectors stored
    count: RwLock<usize>,
    /// Free slots (indices of removed vectors that can be reused)
    free_slots: RwLock<Vec<usize>>,
}

impl VectorStore {
    /// Creates a new vector store with the specified dimension.
    ///
    /// # Arguments
    ///
    /// * `dimension` - The dimension of vectors to store
    /// * `initial_capacity` - Initial number of vectors to pre-allocate
    #[must_use]
    pub fn new(dimension: usize, initial_capacity: usize) -> Self {
        Self {
            buffer: RwLock::new(Vec::with_capacity(dimension * initial_capacity)),
            dimension,
            count: RwLock::new(0),
            free_slots: RwLock::new(Vec::new()),
        }
    }

    /// Returns the vector dimension.
    #[must_use]
    pub fn dimension(&self) -> usize {
        self.dimension
    }

    /// Returns the number of vectors stored.
    #[must_use]
    pub fn len(&self) -> usize {
        *self.count.read()
    }

    /// Returns true if the store is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Inserts a vector and returns its index.
    ///
    /// # Arguments
    ///
    /// * `vector` - The vector to insert (must match dimension)
    ///
    /// # Returns
    ///
    /// The index of the inserted vector.
    ///
    /// # Panics
    ///
    /// Panics if the vector dimension doesn't match.
    pub fn insert(&self, vector: &[f32]) -> usize {
        assert_eq!(
            vector.len(),
            self.dimension,
            "Vector dimension mismatch: expected {}, got {}",
            self.dimension,
            vector.len()
        );

        let mut free_slots = self.free_slots.write();
        let mut buffer = self.buffer.write();
        let mut count = self.count.write();

        // Reuse a free slot if available
        if let Some(idx) = free_slots.pop() {
            let offset = idx * self.dimension;
            buffer[offset..offset + self.dimension].copy_from_slice(vector);
            return idx;
        }

        // Append to end
        let idx = *count;
        buffer.extend_from_slice(vector);
        *count += 1;

        idx
    }

    /// Retrieves a vector by index.
    ///
    /// # Arguments
    ///
    /// * `idx` - The index of the vector to retrieve
    ///
    /// # Returns
    ///
    /// A copy of the vector, or `None` if the index is invalid.
    #[must_use]
    pub fn get(&self, idx: usize) -> Option<Vec<f32>> {
        let buffer = self.buffer.read();
        let offset = idx * self.dimension;

        if offset + self.dimension <= buffer.len() {
            Some(buffer[offset..offset + self.dimension].to_vec())
        } else {
            None
        }
    }

    /// Gets a reference to a vector's data for in-place computation.
    ///
    /// This is more efficient than `get()` when you only need to read the vector.
    ///
    /// # Safety
    ///
    /// The returned slice is only valid while the read lock is held.
    /// Do not store the slice beyond the scope where it's used.
    #[must_use]
    pub fn get_slice(&self, idx: usize) -> Option<VectorRef<'_>> {
        let buffer = self.buffer.read();
        let offset = idx * self.dimension;

        if offset + self.dimension <= buffer.len() {
            Some(VectorRef {
                guard: buffer,
                offset,
                dimension: self.dimension,
            })
        } else {
            None
        }
    }

    /// Removes a vector by index (marks slot as free for reuse).
    ///
    /// # Arguments
    ///
    /// * `idx` - The index of the vector to remove
    ///
    /// # Returns
    ///
    /// `true` if the vector was removed, `false` if the index was invalid.
    pub fn remove(&self, idx: usize) -> bool {
        let buffer = self.buffer.read();
        let offset = idx * self.dimension;

        if offset + self.dimension <= buffer.len() {
            drop(buffer);
            let mut free_slots = self.free_slots.write();
            free_slots.push(idx);
            true
        } else {
            false
        }
    }

    /// Updates a vector at the given index.
    ///
    /// # Arguments
    ///
    /// * `idx` - The index of the vector to update
    /// * `vector` - The new vector data
    ///
    /// # Returns
    ///
    /// `true` if the vector was updated, `false` if the index was invalid.
    ///
    /// # Panics
    ///
    /// Panics if the vector dimension doesn't match.
    pub fn update(&self, idx: usize, vector: &[f32]) -> bool {
        assert_eq!(
            vector.len(),
            self.dimension,
            "Vector dimension mismatch: expected {}, got {}",
            self.dimension,
            vector.len()
        );

        let mut buffer = self.buffer.write();
        let offset = idx * self.dimension;

        if offset + self.dimension <= buffer.len() {
            buffer[offset..offset + self.dimension].copy_from_slice(vector);
            true
        } else {
            false
        }
    }

    /// Returns the memory usage in bytes.
    #[must_use]
    pub fn memory_usage(&self) -> usize {
        let buffer = self.buffer.read();
        buffer.capacity() * std::mem::size_of::<f32>()
    }

    /// Prefetches a vector into CPU cache.
    ///
    /// Call this ahead of time for vectors you'll access soon.
    #[inline]
    pub fn prefetch(&self, idx: usize) {
        let buffer = self.buffer.read();
        let offset = idx * self.dimension;

        if offset < buffer.len() {
            #[cfg(target_arch = "x86_64")]
            // SAFETY (EPIC-032/US-003): _mm_prefetch is safe when:
            // 1. Target has x86_64 architecture (cfg guard)
            // 2. Pointer is within bounds (offset < buffer.len() check above)
            unsafe {
                use std::arch::x86_64::{_mm_prefetch, _MM_HINT_T0};
                let ptr = buffer.as_ptr().add(offset);
                _mm_prefetch(ptr.cast::<i8>(), _MM_HINT_T0);
            }

            #[cfg(target_arch = "aarch64")]
            {
                // Prefetch on aarch64 is currently unstable in std::arch::aarch64
                // Skipping for now on stable Rust to ensure compatibility
            }
        }
    }
}

/// A reference to a vector slice with automatic lock management.
pub struct VectorRef<'a> {
    guard: parking_lot::RwLockReadGuard<'a, Vec<f32>>,
    offset: usize,
    dimension: usize,
}

impl VectorRef<'_> {
    /// Returns the vector as a slice.
    #[must_use]
    pub fn as_slice(&self) -> &[f32] {
        // SAFETY: `from_raw_parts` requires a valid pointer/length pair.
        // - Condition 1: `offset` and `dimension` were bounds-checked in `get_slice`.
        // - Condition 2: `guard` keeps buffer allocation alive for this borrow.
        // Reason: Expose zero-copy read access without allocating `Vec<f32>`.
        unsafe {
            let ptr = self.guard.as_ptr().add(self.offset);
            std::slice::from_raw_parts(ptr, self.dimension)
        }
    }
}

impl std::ops::Deref for VectorRef<'_> {
    type Target = [f32];

    fn deref(&self) -> &Self::Target {
        self.as_slice()
    }
}
