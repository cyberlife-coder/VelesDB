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
            unsafe {
                use std::arch::x86_64::{_mm_prefetch, _MM_HINT_T0};
                let ptr = buffer.as_ptr().add(offset);
                _mm_prefetch(ptr.cast::<i8>(), _MM_HINT_T0);
            }

            #[cfg(target_arch = "aarch64")]
            unsafe {
                use std::arch::aarch64::_prefetch;
                let ptr = buffer.as_ptr().add(offset);
                _prefetch(ptr.cast::<i8>(), 0, 3); // PLDL1KEEP
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
        // SAFETY: The guard ensures the buffer is valid for the lifetime 'a
        // and offset/dimension were validated during construction
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vector_store_new() {
        let store = VectorStore::new(768, 1000);
        assert_eq!(store.dimension(), 768);
        assert_eq!(store.len(), 0);
        assert!(store.is_empty());
    }

    #[test]
    fn test_vector_store_insert_and_get() {
        let store = VectorStore::new(4, 10);
        let vec1 = vec![1.0, 2.0, 3.0, 4.0];
        let vec2 = vec![5.0, 6.0, 7.0, 8.0];

        let idx1 = store.insert(&vec1);
        let idx2 = store.insert(&vec2);

        assert_eq!(idx1, 0);
        assert_eq!(idx2, 1);
        assert_eq!(store.len(), 2);

        assert_eq!(store.get(idx1), Some(vec1));
        assert_eq!(store.get(idx2), Some(vec2));
    }

    #[test]
    fn test_vector_store_get_slice() {
        let store = VectorStore::new(3, 10);
        let vec1 = vec![1.0, 2.0, 3.0];

        let idx = store.insert(&vec1);
        let slice_ref = store.get_slice(idx).unwrap();

        assert_eq!(slice_ref.as_slice(), &[1.0, 2.0, 3.0]);
        assert_eq!(&*slice_ref, &[1.0, 2.0, 3.0]); // Test Deref
    }

    #[test]
    fn test_vector_store_update() {
        let store = VectorStore::new(3, 10);
        let vec1 = vec![1.0, 2.0, 3.0];
        let vec2 = vec![4.0, 5.0, 6.0];

        let idx = store.insert(&vec1);
        assert!(store.update(idx, &vec2));
        assert_eq!(store.get(idx), Some(vec2));
    }

    #[test]
    fn test_vector_store_remove_and_reuse() {
        let store = VectorStore::new(2, 10);
        let vec1 = vec![1.0, 2.0];
        let vec2 = vec![3.0, 4.0];
        let vec3 = vec![5.0, 6.0];

        let idx1 = store.insert(&vec1);
        let idx2 = store.insert(&vec2);

        // Remove first vector
        assert!(store.remove(idx1));

        // Insert new vector should reuse slot
        let idx3 = store.insert(&vec3);
        assert_eq!(idx3, idx1); // Should reuse slot 0

        assert_eq!(store.get(idx2), Some(vec2));
        assert_eq!(store.get(idx3), Some(vec3));
    }

    #[test]
    fn test_vector_store_invalid_index() {
        let store = VectorStore::new(3, 10);
        assert!(store.get(0).is_none());
        assert!(store.get(100).is_none());
        assert!(!store.remove(100));
        assert!(!store.update(100, &[1.0, 2.0, 3.0]));
    }

    #[test]
    #[should_panic(expected = "Vector dimension mismatch")]
    fn test_vector_store_dimension_mismatch_insert() {
        let store = VectorStore::new(3, 10);
        store.insert(&[1.0, 2.0]); // Wrong dimension
    }

    #[test]
    #[should_panic(expected = "Vector dimension mismatch")]
    fn test_vector_store_dimension_mismatch_update() {
        let store = VectorStore::new(3, 10);
        let idx = store.insert(&[1.0, 2.0, 3.0]);
        store.update(idx, &[1.0, 2.0]); // Wrong dimension
    }

    #[test]
    fn test_vector_store_memory_usage() {
        let store = VectorStore::new(768, 1000);
        // Pre-allocated capacity should be 768 * 1000 * 4 bytes = ~3MB
        let usage = store.memory_usage();
        assert!(usage >= 768 * 1000 * 4);
    }

    #[test]
    fn test_vector_store_prefetch() {
        let store = VectorStore::new(4, 10);
        let idx = store.insert(&[1.0, 2.0, 3.0, 4.0]);

        // Prefetch should not panic
        store.prefetch(idx);
        store.prefetch(100); // Invalid index should be handled gracefully
    }
}
