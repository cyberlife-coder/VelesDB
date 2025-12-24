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

    /// Adds a vector to the storage.
    ///
    /// # Panics
    ///
    /// Panics if vector dimension doesn't match.
    pub fn push(&mut self, vector: &[f32]) {
        assert_eq!(
            vector.len(),
            self.dimension,
            "Vector dimension mismatch: expected {}, got {}",
            self.dimension,
            vector.len()
        );

        if self.count >= self.capacity {
            self.grow();
        }

        let offset = self.count * self.dimension;
        // SAFETY: We've ensured capacity and bounds
        unsafe {
            ptr::copy_nonoverlapping(vector.as_ptr(), self.data.add(offset), self.dimension);
        }
        self.count += 1;
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
            return None;
        }

        let offset = index * self.dimension;
        // SAFETY: Index is within bounds
        Some(unsafe { std::slice::from_raw_parts(self.data.add(offset), self.dimension) })
    }

    /// Gets a vector by index (unchecked).
    ///
    /// # Safety
    ///
    /// Caller must ensure index < count.
    #[inline]
    #[must_use]
    pub unsafe fn get_unchecked(&self, index: usize) -> &[f32] {
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

    /// Grows the internal buffer by 2x.
    #[allow(clippy::cast_ptr_alignment)] // Layout is 64-byte aligned
    fn grow(&mut self) {
        let new_capacity = self.capacity * 2;
        let old_layout = Self::layout(self.dimension, self.capacity);
        let new_layout = Self::layout(self.dimension, new_capacity);

        // SAFETY: Layouts are valid
        let new_data = unsafe { alloc(new_layout).cast::<f32>() };

        assert!(!new_data.is_null(), "Failed to grow ContiguousVectors");

        // Copy existing data
        let copy_size = self.count * self.dimension;
        unsafe {
            ptr::copy_nonoverlapping(self.data, new_data, copy_size);
            dealloc(self.data.cast::<u8>(), old_layout);
        }

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
}
