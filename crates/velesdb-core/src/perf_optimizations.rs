//! Performance optimizations module for ultra-fast vector operations.
//!
//! This module provides:
//! - **Contiguous vector storage**: Cache-friendly memory layout
//! - **Prefetch hints**: CPU cache warming for HNSW traversal
//! - **Batch distance computation**: SIMD-optimized batch operations
//!
//! # Performance Targets
//!
//! - Bulk import: 50K+ vectors/sec at 768D
//! - Search latency: < 1ms for 1M vectors
//! - Memory efficiency: 50% reduction with FP16

use std::alloc::{alloc, dealloc, Layout};
use std::fmt;
use std::ptr;

// =============================================================================
// Contiguous Vector Storage (Cache-Optimized)
// =============================================================================

/// Contiguous memory layout for vectors (cache-friendly).
///
/// Stores all vectors in a single contiguous buffer to maximize
/// cache locality and enable SIMD prefetching.
///
/// # Memory Layout
///
/// ```text
/// [v0_d0, v0_d1, ..., v0_dn, v1_d0, v1_d1, ..., v1_dn, ...]
/// ```
pub struct ContiguousVectors {
    /// Raw contiguous data buffer
    data: *mut f32,
    /// Vector dimension
    dimension: usize,
    /// Number of vectors stored
    count: usize,
    /// Allocated capacity (number of vectors)
    capacity: usize,
}

// SAFETY: ContiguousVectors owns its data and doesn't share mutable access
unsafe impl Send for ContiguousVectors {}
unsafe impl Sync for ContiguousVectors {}

impl fmt::Debug for ContiguousVectors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ContiguousVectors")
            .field("dimension", &self.dimension)
            .field("count", &self.count)
            .field("capacity", &self.capacity)
            .finish_non_exhaustive()
    }
}

impl ContiguousVectors {
    /// Creates a new `ContiguousVectors` with the given dimension and initial capacity.
    ///
    /// # Arguments
    ///
    /// * `dimension` - Vector dimension
    /// * `capacity` - Initial capacity (number of vectors)
    ///
    /// # Panics
    ///
    /// Panics if dimension is 0 or allocation fails.
    #[must_use]
    #[allow(clippy::cast_ptr_alignment)] // Layout is 64-byte aligned
    pub fn new(dimension: usize, capacity: usize) -> Self {
        assert!(dimension > 0, "Dimension must be > 0");

        let capacity = capacity.max(16); // Minimum 16 vectors
        let layout = Self::layout(dimension, capacity);

        // SAFETY: Layout is valid (non-zero, aligned)
        let data = unsafe { alloc(layout).cast::<f32>() };

        assert!(!data.is_null(), "Failed to allocate ContiguousVectors");

        Self {
            data,
            dimension,
            count: 0,
            capacity,
        }
    }

    /// Returns the memory layout for the given dimension and capacity.
    fn layout(dimension: usize, capacity: usize) -> Layout {
        let size = dimension * capacity * std::mem::size_of::<f32>();
        let align = 64; // Cache line alignment for optimal prefetch
        Layout::from_size_align(size.max(64), align).expect("Invalid layout")
    }

    /// Returns the dimension of stored vectors.
    #[inline]
    #[must_use]
    pub const fn dimension(&self) -> usize {
        self.dimension
    }

    /// Returns the number of vectors stored.
    #[inline]
    #[must_use]
    pub const fn len(&self) -> usize {
        self.count
    }

    /// Returns true if no vectors are stored.
    #[inline]
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Returns the capacity (max vectors before reallocation).
    #[inline]
    #[must_use]
    pub const fn capacity(&self) -> usize {
        self.capacity
    }

    /// Returns total memory usage in bytes.
    #[inline]
    #[must_use]
    pub const fn memory_bytes(&self) -> usize {
        self.capacity * self.dimension * std::mem::size_of::<f32>()
    }

    /// Ensures the storage has capacity for at least `required_capacity` vectors.
    pub fn ensure_capacity(&mut self, required_capacity: usize) {
        if required_capacity > self.capacity {
            let new_capacity = required_capacity.max(self.capacity * 2);
            self.resize(new_capacity);
        }
    }

    /// Inserts a vector at a specific index.
    ///
    /// Automatically grows capacity if needed.
    /// Note: This allows sparse population. Uninitialized slots contain undefined data (or 0.0 if alloc gave zeroed memory).
    ///
    /// # Panics
    ///
    /// Panics if vector dimension doesn't match.
    pub fn insert_at(&mut self, index: usize, vector: &[f32]) {
        assert_eq!(
            vector.len(),
            self.dimension,
            "Vector dimension mismatch: expected {}, got {}",
            self.dimension,
            vector.len()
        );

        self.ensure_capacity(index + 1);

        let offset = index * self.dimension;
        // SAFETY: We ensured capacity covers index
        unsafe {
            ptr::copy_nonoverlapping(vector.as_ptr(), self.data.add(offset), self.dimension);
        }

        // Update count if we're extending the "used" range
        if index >= self.count {
            self.count = index + 1;
        }
    }

    /// Adds a vector to the storage.
    ///
    /// # Panics
    ///
    /// Panics if vector dimension doesn't match.
    pub fn push(&mut self, vector: &[f32]) {
        self.insert_at(self.count, vector);
    }

    /// Adds multiple vectors in batch (optimized).
    ///
    /// # Arguments
    ///
    /// * `vectors` - Iterator of vectors to add
    ///
    /// # Returns
    ///
    /// Number of vectors added.
    pub fn push_batch<'a>(&mut self, vectors: impl Iterator<Item = &'a [f32]>) -> usize {
        let mut added = 0;
        for vector in vectors {
            self.push(vector);
            added += 1;
        }
        added
    }

    /// Gets a vector by index.
    ///
    /// # Returns
    ///
    /// Slice to the vector data, or `None` if index is out of bounds.
    #[inline]
    #[must_use]
    pub fn get(&self, index: usize) -> Option<&[f32]> {
        if index >= self.count {
            // Note: In sparse mode, index < count doesn't guarantee it was initialized,
            // but for HNSW dense IDs it typically does.
            return None;
        }

        let offset = index * self.dimension;
        // SAFETY: Index is within bounds (checked against count, which is <= capacity)
        Some(unsafe { std::slice::from_raw_parts(self.data.add(offset), self.dimension) })
    }

    /// Gets a vector by index (unchecked).
    ///
    /// # Safety
    ///
    /// Caller must ensure `index < self.len()`.
    ///
    /// # Debug Assertions
    ///
    /// In debug builds, this function will panic if `index >= self.len()`.
    /// This catches bugs early during development without impacting release performance.
    #[inline]
    #[must_use]
    pub unsafe fn get_unchecked(&self, index: usize) -> &[f32] {
        debug_assert!(
            index < self.count,
            "index out of bounds: index={index}, count={}",
            self.count
        );
        let offset = index * self.dimension;
        std::slice::from_raw_parts(self.data.add(offset), self.dimension)
    }

    /// Prefetches a vector for upcoming access.
    ///
    /// This hints the CPU to load the vector into L2 cache.
    #[inline]
    pub fn prefetch(&self, index: usize) {
        if index < self.count {
            let offset = index * self.dimension;
            let ptr = unsafe { self.data.add(offset) };

            #[cfg(target_arch = "x86_64")]
            unsafe {
                use std::arch::x86_64::_mm_prefetch;
                // Prefetch for read, into L2 cache
                _mm_prefetch(ptr.cast::<i8>(), std::arch::x86_64::_MM_HINT_T1);
            }

            // aarch64 prefetch requires nightly (stdarch_aarch64_prefetch)
            // For now, we skip prefetch on ARM64 until the feature is stabilized
            #[cfg(not(target_arch = "x86_64"))]
            let _ = ptr;
        }
    }

    /// Prefetches multiple vectors for batch processing.
    #[inline]
    pub fn prefetch_batch(&self, indices: &[usize]) {
        for &idx in indices {
            self.prefetch(idx);
        }
    }

    /// Resizes the internal buffer.
    ///
    /// # P2 Audit + PERF-002: Panic-Safety with RAII Guard
    ///
    /// This function uses `AllocGuard` for panic-safe allocation:
    /// 1. New buffer is allocated via RAII guard (auto-freed on panic)
    /// 2. Data is copied to new buffer
    /// 3. Guard ownership is transferred (no auto-free)
    /// 4. Old buffer is deallocated
    /// 5. State is updated atomically
    ///
    /// If panic occurs during copy, the guard ensures new buffer is freed.
    #[allow(clippy::cast_ptr_alignment)] // Layout is 64-byte aligned
    fn resize(&mut self, new_capacity: usize) {
        use crate::alloc_guard::AllocGuard;

        if new_capacity <= self.capacity {
            return;
        }

        let old_layout = Self::layout(self.dimension, self.capacity);
        let new_layout = Self::layout(self.dimension, new_capacity);

        // Step 1: Allocate new buffer with RAII guard (PERF-002)
        // If panic occurs before into_raw(), memory is automatically freed
        let guard = AllocGuard::new(new_layout).unwrap_or_else(|| {
            panic!(
                "Failed to allocate {} bytes for ContiguousVectors resize",
                new_layout.size()
            )
        });

        let new_data: *mut f32 = guard.cast();

        // Step 2: Copy existing data to new buffer
        // If this panics, guard drops and frees new_data automatically
        let copy_count = self.count;
        if copy_count > 0 {
            let copy_size = copy_count * self.dimension;
            // SAFETY: Both pointers are valid, non-overlapping, and properly aligned
            unsafe {
                ptr::copy_nonoverlapping(self.data, new_data, copy_size);
            }
        }

        // Step 3: Transfer ownership - guard won't free on drop anymore
        let _ = guard.into_raw();

        // Step 4: Deallocate old buffer
        // SAFETY: self.data was allocated with old_layout
        unsafe {
            dealloc(self.data.cast::<u8>(), old_layout);
        }

        // Step 5: Update state (all-or-nothing)
        self.data = new_data;
        self.capacity = new_capacity;
    }

    /// Computes dot product with another vector using SIMD.
    #[inline]
    #[must_use]
    pub fn dot_product(&self, index: usize, query: &[f32]) -> Option<f32> {
        let vector = self.get(index)?;
        Some(crate::simd_avx512::dot_product_auto(vector, query))
    }

    /// Prefetch distance for cache warming.
    const PREFETCH_DISTANCE: usize = 4;

    /// Computes batch dot products with a query vector.
    ///
    /// This is optimized for HNSW search with prefetching.
    #[must_use]
    pub fn batch_dot_products(&self, indices: &[usize], query: &[f32]) -> Vec<f32> {
        let mut results = Vec::with_capacity(indices.len());

        for (i, &idx) in indices.iter().enumerate() {
            // Prefetch upcoming vectors
            if i + Self::PREFETCH_DISTANCE < indices.len() {
                self.prefetch(indices[i + Self::PREFETCH_DISTANCE]);
            }

            if let Some(score) = self.dot_product(idx, query) {
                results.push(score);
            }
        }

        results
    }
}

impl Drop for ContiguousVectors {
    fn drop(&mut self) {
        if !self.data.is_null() {
            let layout = Self::layout(self.dimension, self.capacity);
            // SAFETY: data was allocated with this layout
            unsafe {
                dealloc(self.data.cast::<u8>(), layout);
            }
        }
    }
}

// =============================================================================
// Batch Distance Computation
// =============================================================================

/// Computes multiple dot products in a single pass (cache-optimized).
///
/// Uses prefetching and SIMD for maximum throughput.
#[must_use]
pub fn batch_dot_products_simd(vectors: &[&[f32]], query: &[f32]) -> Vec<f32> {
    vectors
        .iter()
        .map(|v| crate::simd_avx512::dot_product_auto(v, query))
        .collect()
}

/// Computes multiple cosine similarities in a single pass.
#[must_use]
pub fn batch_cosine_similarities(vectors: &[&[f32]], query: &[f32]) -> Vec<f32> {
    vectors
        .iter()
        .map(|v| crate::simd_avx512::cosine_similarity_auto(v, query))
        .collect()
}

// =============================================================================
// Tests (TDD - Tests First!)
// =============================================================================

#[cfg(test)]
#[allow(clippy::cast_precision_loss)]
mod tests {
    use super::*;

    const EPSILON: f32 = 1e-5;

    // =========================================================================
    // ContiguousVectors Tests
    // =========================================================================

    #[test]
    fn test_contiguous_vectors_new() {
        let cv = ContiguousVectors::new(768, 100);
        assert_eq!(cv.dimension(), 768);
        assert_eq!(cv.len(), 0);
        assert!(cv.is_empty());
        assert!(cv.capacity() >= 100);
    }

    #[test]
    fn test_contiguous_vectors_push() {
        let mut cv = ContiguousVectors::new(3, 10);
        let v1 = vec![1.0, 2.0, 3.0];
        let v2 = vec![4.0, 5.0, 6.0];

        cv.push(&v1);
        assert_eq!(cv.len(), 1);

        cv.push(&v2);
        assert_eq!(cv.len(), 2);

        let retrieved = cv.get(0).unwrap();
        assert_eq!(retrieved, &v1[..]);

        let retrieved = cv.get(1).unwrap();
        assert_eq!(retrieved, &v2[..]);
    }

    #[test]
    fn test_contiguous_vectors_push_batch() {
        let mut cv = ContiguousVectors::new(128, 100);
        let vectors: Vec<Vec<f32>> = (0..50)
            .map(|i| (0..128).map(|j| (i * 128 + j) as f32).collect())
            .collect();

        let refs: Vec<&[f32]> = vectors.iter().map(Vec::as_slice).collect();
        let added = cv.push_batch(refs.into_iter());

        assert_eq!(added, 50);
        assert_eq!(cv.len(), 50);
    }

    #[test]
    fn test_contiguous_vectors_grow() {
        let mut cv = ContiguousVectors::new(64, 16);
        let vector: Vec<f32> = (0..64).map(|i| i as f32).collect();

        // Push more than initial capacity
        for _ in 0..50 {
            cv.push(&vector);
        }

        assert_eq!(cv.len(), 50);
        assert!(cv.capacity() >= 50);

        // Verify data integrity
        for i in 0..50 {
            let retrieved = cv.get(i).unwrap();
            assert_eq!(retrieved, &vector[..]);
        }
    }

    #[test]
    fn test_contiguous_vectors_get_out_of_bounds() {
        let cv = ContiguousVectors::new(3, 10);
        assert!(cv.get(0).is_none());
        assert!(cv.get(100).is_none());
    }

    #[test]
    #[should_panic(expected = "dimension mismatch")]
    fn test_contiguous_vectors_dimension_mismatch() {
        let mut cv = ContiguousVectors::new(3, 10);
        cv.push(&[1.0, 2.0]); // Wrong dimension
    }

    #[test]
    fn test_contiguous_vectors_memory_bytes() {
        let cv = ContiguousVectors::new(768, 1000);
        let expected = 1000 * 768 * 4; // capacity * dimension * sizeof(f32)
        assert!(cv.memory_bytes() >= expected);
    }

    #[test]
    fn test_contiguous_vectors_prefetch() {
        let mut cv = ContiguousVectors::new(64, 100);
        for i in 0..50 {
            let v: Vec<f32> = (0..64).map(|j| (i * 64 + j) as f32).collect();
            cv.push(&v);
        }

        // Should not panic
        cv.prefetch(0);
        cv.prefetch(25);
        cv.prefetch(49);
        cv.prefetch(100); // Out of bounds - should be no-op
    }

    #[test]
    fn test_contiguous_vectors_dot_product() {
        let mut cv = ContiguousVectors::new(3, 10);
        cv.push(&[1.0, 0.0, 0.0]);
        cv.push(&[0.0, 1.0, 0.0]);

        let query = vec![1.0, 0.0, 0.0];

        let dp0 = cv.dot_product(0, &query).unwrap();
        assert!((dp0 - 1.0).abs() < EPSILON);

        let dp1 = cv.dot_product(1, &query).unwrap();
        assert!((dp1 - 0.0).abs() < EPSILON);
    }

    #[test]
    fn test_contiguous_vectors_batch_dot_products() {
        let mut cv = ContiguousVectors::new(64, 100);

        // Add normalized vectors
        for i in 0..50 {
            let mut v: Vec<f32> = (0..64).map(|j| ((i + j) % 10) as f32).collect();
            let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
            if norm > 0.0 {
                for x in &mut v {
                    *x /= norm;
                }
            }
            cv.push(&v);
        }

        let query: Vec<f32> = (0..64).map(|i| i as f32 / 64.0).collect();
        let indices: Vec<usize> = (0..50).collect();

        let results = cv.batch_dot_products(&indices, &query);
        assert_eq!(results.len(), 50);
    }

    // =========================================================================
    // Batch Distance Tests
    // =========================================================================

    #[test]
    fn test_batch_dot_products_simd() {
        let v1 = vec![1.0, 0.0, 0.0];
        let v2 = vec![0.0, 1.0, 0.0];
        let v3 = vec![0.5, 0.5, 0.0];
        let query = vec![1.0, 0.0, 0.0];

        let vectors: Vec<&[f32]> = vec![&v1, &v2, &v3];
        let results = batch_dot_products_simd(&vectors, &query);

        assert_eq!(results.len(), 3);
        assert!((results[0] - 1.0).abs() < EPSILON);
        assert!((results[1] - 0.0).abs() < EPSILON);
        assert!((results[2] - 0.5).abs() < EPSILON);
    }

    #[test]
    fn test_batch_cosine_similarities() {
        let v1 = vec![1.0, 0.0, 0.0];
        let v2 = vec![0.0, 1.0, 0.0];
        let query = vec![1.0, 0.0, 0.0];

        let vectors: Vec<&[f32]> = vec![&v1, &v2];
        let results = batch_cosine_similarities(&vectors, &query);

        assert_eq!(results.len(), 2);
        assert!((results[0] - 1.0).abs() < EPSILON); // Same direction
        assert!((results[1] - 0.0).abs() < EPSILON); // Orthogonal
    }

    // =========================================================================
    // Performance-Critical Tests
    // =========================================================================

    #[test]
    fn test_contiguous_large_dimension() {
        // Test with BERT-like dimensions (768D)
        let mut cv = ContiguousVectors::new(768, 1000);

        for i in 0..100 {
            let v: Vec<f32> = (0..768).map(|j| ((i + j) % 100) as f32 / 100.0).collect();
            cv.push(&v);
        }

        assert_eq!(cv.len(), 100);

        // Verify random access works
        let v50 = cv.get(50).unwrap();
        assert_eq!(v50.len(), 768);
    }

    #[test]
    fn test_contiguous_gpt4_dimension() {
        // Test with GPT-4 dimensions (1536D)
        let mut cv = ContiguousVectors::new(1536, 100);

        for i in 0..20 {
            let v: Vec<f32> = (0..1536).map(|j| ((i + j) % 100) as f32 / 100.0).collect();
            cv.push(&v);
        }

        assert_eq!(cv.len(), 20);
        assert_eq!(cv.dimension(), 1536);
    }

    // =========================================================================
    // Safety: get_unchecked bounds check tests (TDD)
    // =========================================================================

    #[test]
    fn test_get_unchecked_valid_index() {
        // Arrange
        let mut cv = ContiguousVectors::new(3, 10);
        cv.push(&[1.0, 2.0, 3.0]);
        cv.push(&[4.0, 5.0, 6.0]);

        // Act - Valid indices should work
        let v0 = unsafe { cv.get_unchecked(0) };
        let v1 = unsafe { cv.get_unchecked(1) };

        // Assert
        assert_eq!(v0, &[1.0, 2.0, 3.0]);
        assert_eq!(v1, &[4.0, 5.0, 6.0]);
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "index out of bounds")]
    fn test_get_unchecked_panics_on_invalid_index_in_debug() {
        // Arrange
        let mut cv = ContiguousVectors::new(3, 10);
        cv.push(&[1.0, 2.0, 3.0]);

        // Act - Out of bounds index should panic in debug mode
        let _ = unsafe { cv.get_unchecked(5) };
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "index out of bounds")]
    fn test_get_unchecked_panics_on_boundary_index_in_debug() {
        // Arrange
        let mut cv = ContiguousVectors::new(3, 10);
        cv.push(&[1.0, 2.0, 3.0]);
        cv.push(&[4.0, 5.0, 6.0]);

        // Act - Index == count should panic (off by one)
        let _ = unsafe { cv.get_unchecked(2) };
    }

    // =========================================================================
    // P2 Audit: Resize panic-safety tests
    // =========================================================================

    #[test]
    fn test_resize_preserves_data_integrity() {
        // Arrange
        let mut cv = ContiguousVectors::new(64, 16);
        let vectors: Vec<Vec<f32>> = (0..10)
            .map(|i| (0..64).map(|j| (i * 64 + j) as f32).collect())
            .collect();

        for v in &vectors {
            cv.push(v);
        }

        // Act - Force resize by adding more vectors
        for i in 10..100 {
            let v: Vec<f32> = (0..64).map(|j| (i * 64 + j) as f32).collect();
            cv.push(&v);
        }

        // Assert - Original vectors should be intact
        for (i, expected) in vectors.iter().enumerate() {
            let actual = cv.get(i).expect("Vector should exist");
            assert_eq!(
                actual,
                expected.as_slice(),
                "Vector {i} corrupted after resize"
            );
        }
    }

    #[test]
    fn test_resize_multiple_times() {
        // Arrange - Start with minimal capacity
        let mut cv = ContiguousVectors::new(128, 16);

        // Act - Trigger multiple resizes
        for i in 0..500 {
            let v: Vec<f32> = (0..128).map(|j| (i * 128 + j) as f32).collect();
            cv.push(&v);
        }

        // Assert
        assert_eq!(cv.len(), 500);
        assert!(cv.capacity() >= 500);

        // Verify first and last vectors
        let first = cv.get(0).unwrap();
        assert!((first[0] - 0.0).abs() < f32::EPSILON);

        let last = cv.get(499).unwrap();
        #[allow(clippy::cast_precision_loss)]
        let expected = (499 * 128) as f32;
        assert!((last[0] - expected).abs() < f32::EPSILON);
    }

    #[test]
    fn test_drop_after_resize_no_leak() {
        // Arrange - Create and resize multiple times
        for _ in 0..10 {
            let mut cv = ContiguousVectors::new(256, 8);

            // Trigger multiple resizes
            for i in 0..100 {
                let v: Vec<f32> = (0..256).map(|j| (i + j) as f32).collect();
                cv.push(&v);
            }

            // cv is dropped here - should not leak memory
        }

        // If we get here without memory issues, the test passes
        // Note: In a real scenario, use tools like valgrind or miri to verify
    }

    #[test]
    fn test_ensure_capacity_idempotent() {
        // Arrange
        let mut cv = ContiguousVectors::new(64, 100);
        cv.push(&vec![1.0; 64]);

        let initial_capacity = cv.capacity();

        // Act - Call ensure_capacity multiple times with same value
        cv.ensure_capacity(50);
        cv.ensure_capacity(50);
        cv.ensure_capacity(50);

        // Assert - Capacity should not change
        assert_eq!(cv.capacity(), initial_capacity);
        assert_eq!(cv.len(), 1);
    }
}
