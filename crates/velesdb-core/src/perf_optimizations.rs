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
//!
//! # Safety (EPIC-032/US-002)
//!
//! `ContiguousVectors` uses `NonNull<f32>` to encode non-nullness at the type level,
//! eliminating null pointer checks and making invariants explicit. Memory is managed
//! via RAII with `AllocGuard` for panic-safe resize operations.

use std::alloc::{alloc, dealloc, Layout};
use std::fmt;
use std::ptr::{self, NonNull};

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
///
/// # Safety Invariants (EPIC-032/US-002)
///
/// - `data` is always non-null (enforced by `NonNull`)
/// - `data` points to memory allocated with 64-byte alignment
/// - `capacity * dimension * sizeof(f32)` bytes are always allocated
/// - `count <= capacity` is always maintained
pub struct ContiguousVectors {
    /// Non-null contiguous data buffer (EPIC-032/US-002: type-level non-null guarantee)
    data: NonNull<f32>,
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

        // SAFETY: Layout is valid (non-zero size due to dimension > 0 and capacity >= 16)
        let ptr = unsafe { alloc(layout) };

        // EPIC-032/US-002: Use NonNull for type-level non-null guarantee
        let data = NonNull::new(ptr.cast::<f32>())
            .expect("Failed to allocate ContiguousVectors: out of memory");

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
        // SAFETY: We ensured capacity covers index, data is non-null (NonNull invariant)
        unsafe {
            ptr::copy_nonoverlapping(
                vector.as_ptr(),
                self.data.as_ptr().add(offset),
                self.dimension,
            );
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
        // data is non-null (NonNull invariant)
        Some(unsafe { std::slice::from_raw_parts(self.data.as_ptr().add(offset), self.dimension) })
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
        // SAFETY: Caller guarantees index < count, data is non-null (NonNull invariant)
        std::slice::from_raw_parts(self.data.as_ptr().add(offset), self.dimension)
    }

    /// Prefetches a vector for upcoming access.
    ///
    /// This hints the CPU to load the vector into L2 cache.
    #[inline]
    pub fn prefetch(&self, index: usize) {
        if index < self.count {
            let offset = index * self.dimension;
            // SAFETY: index < count implies valid offset, data is non-null (NonNull invariant)
            let ptr = unsafe { self.data.as_ptr().add(offset) };

            #[cfg(target_arch = "x86_64")]
            // SAFETY: _mm_prefetch is a hint instruction that cannot cause undefined behavior.
            // The pointer is valid (derived from data.as_ptr() with bounds-checked offset).
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

        // EPIC-032/US-002: Use NonNull for type-level guarantee
        let new_data = NonNull::new(guard.cast::<f32>()).expect("AllocGuard returned null pointer");

        // Step 2: Copy existing data to new buffer
        // If this panics, guard drops and frees new_data automatically
        let copy_count = self.count;
        if copy_count > 0 {
            let copy_size = copy_count * self.dimension;
            // SAFETY: Both pointers are valid (NonNull), non-overlapping, and properly aligned
            unsafe {
                ptr::copy_nonoverlapping(self.data.as_ptr(), new_data.as_ptr(), copy_size);
            }
        }

        // Step 3: Transfer ownership - guard won't free on drop anymore
        let _ = guard.into_raw();

        // Step 4: Deallocate old buffer
        // SAFETY: self.data was allocated with old_layout, is non-null (NonNull invariant)
        unsafe {
            dealloc(self.data.as_ptr().cast::<u8>(), old_layout);
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
        // EPIC-032/US-002: No null check needed - NonNull guarantees non-null
        let layout = Self::layout(self.dimension, self.capacity);
        // SAFETY: data was allocated with this layout, is non-null (NonNull invariant)
        unsafe {
            dealloc(self.data.as_ptr().cast::<u8>(), layout);
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
