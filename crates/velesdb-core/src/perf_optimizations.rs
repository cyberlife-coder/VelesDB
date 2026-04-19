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

use crate::validation::validate_dimension_match;
use std::alloc::{alloc_zeroed, Layout};
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
    pub(crate) data: NonNull<f32>,
    /// Vector dimension
    pub(crate) dimension: usize,
    /// Number of vectors stored
    pub(crate) count: usize,
    /// Allocated capacity (number of vectors)
    pub(crate) capacity: usize,
}

// SAFETY: `ContiguousVectors` is `Send` because it owns its allocation.
// - Condition 1: The backing buffer is uniquely owned by the struct.
// - Condition 2: Mutation requires `&mut self` or lock-guarded interior access.
// SAFETY: Moving ownership of this container between threads is sound.
unsafe impl Send for ContiguousVectors {}
// SAFETY: `ContiguousVectors` is `Sync` because shared access is read-only.
// - Condition 1: All writes happen through methods requiring mutable or exclusive lock access.
// - Condition 2: Returned shared slices borrow immutably and cannot mutate internal state.
// SAFETY: Concurrent shared references cannot violate aliasing rules.
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
    /// * `dimension` - Vector dimension (must be > 0)
    /// * `capacity` - Initial capacity (number of vectors)
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidDimension`] if `dimension` is 0.
    /// Returns [`Error::AllocationFailed`] if memory allocation fails.
    ///
    /// [`Error::InvalidDimension`]: crate::error::Error::InvalidDimension
    /// [`Error::AllocationFailed`]: crate::error::Error::AllocationFailed
    #[allow(clippy::cast_ptr_alignment)] // Layout is 64-byte aligned
    pub fn new(dimension: usize, capacity: usize) -> crate::error::Result<Self> {
        if dimension == 0 {
            return Err(crate::error::Error::InvalidDimension {
                dimension: 0,
                min: 1,
                max: 65_536,
            });
        }

        let capacity = capacity.max(16); // Minimum 16 vectors
        let layout = Self::layout(dimension, capacity)?;

        // SAFETY: `alloc_zeroed` requires a valid non-zero layout.
        // - Condition 1: `dimension > 0` and `capacity >= 16` guarantee non-zero size.
        // - Condition 2: `layout` is built via `Layout::from_size_align` and therefore valid.
        // SAFETY: Zero-initialized allocation guarantees all f32 slots are 0.0,
        // preventing UB when `insert_at` creates sparse gaps (indices 0..N not all written).
        let ptr = unsafe { alloc_zeroed(layout) };

        // EPIC-032/US-002: Use NonNull for type-level non-null guarantee
        let data = NonNull::new(ptr.cast::<f32>()).ok_or_else(|| {
            crate::error::Error::AllocationFailed(
                "ContiguousVectors: allocator returned null".to_string(),
            )
        })?;

        Ok(Self {
            data,
            dimension,
            count: 0,
            capacity,
        })
    }

    /// Returns the memory layout for the given dimension and capacity.
    ///
    /// # Errors
    ///
    /// Returns [`Error::AllocationFailed`] if the layout parameters are invalid.
    ///
    /// [`Error::AllocationFailed`]: crate::error::Error::AllocationFailed
    pub(crate) fn layout(dimension: usize, capacity: usize) -> crate::error::Result<Layout> {
        let size = dimension
            .checked_mul(capacity)
            .and_then(|s| s.checked_mul(std::mem::size_of::<f32>()))
            .ok_or_else(|| {
                crate::error::Error::AllocationFailed(format!(
                    "Size overflow: {dimension} * {capacity} * {}",
                    std::mem::size_of::<f32>()
                ))
            })?;
        let align = 64; // Cache line alignment for optimal prefetch
        Layout::from_size_align(size.max(64), align)
            .map_err(|e| crate::error::Error::AllocationFailed(format!("Invalid layout: {e}")))
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

    /// Returns the raw contiguous buffer as a flat slice.
    ///
    /// The slice contains all vectors packed sequentially:
    /// `[v0_d0, v0_d1, ..., v1_d0, ...]`.
    /// Useful for GPU upload without copying.
    #[inline]
    #[must_use]
    pub fn as_flat_slice(&self) -> &[f32] {
        if self.count == 0 {
            return &[];
        }
        let total = self.count * self.dimension;
        // SAFETY: All `capacity * dimension` f32s are valid because both initial allocation
        // (`alloc_zeroed`) and resize (`AllocGuard::new_zeroed`) zero-initialize the buffer.
        // `count * dimension <= capacity * dimension`, `data` is non-null per `NonNull`
        // invariant. Even sparse `insert_at` gaps contain valid 0.0 f32 values.
        // - Condition 1: `data` is a valid, aligned `NonNull<f32>` pointer.
        // - Condition 2: `total <= capacity * dimension` ensures the slice is within the allocation.
        // - Condition 3: All bytes in the allocation are initialized (zeroed or written).
        // SAFETY: Zero-copy GPU upload requires a contiguous &[f32] view.
        unsafe { std::slice::from_raw_parts(self.data.as_ptr(), total) }
    }

    /// Gathers vectors at the specified indices into a contiguous flat buffer.
    ///
    /// Returns a new `Vec<f32>` containing the selected vectors packed sequentially.
    /// Useful for GPU upload when only a subset of vectors is needed (e.g., reranking).
    ///
    /// # Important
    ///
    /// Out-of-bounds indices are silently skipped — the result may contain fewer
    /// vectors than `indices.len()`. Callers **must** validate
    /// `result.len() == indices.len() * dimension` before using the result in
    /// positional operations (e.g., `zip` with an ID map), otherwise scores
    /// will be misattributed to wrong IDs.
    #[must_use]
    pub fn gather_flat(&self, indices: &[usize]) -> Vec<f32> {
        let mut result = Vec::with_capacity(indices.len() * self.dimension);
        for &idx in indices {
            if let Some(vec) = self.get(idx) {
                result.extend_from_slice(vec);
            }
        }
        result
    }

    /// Returns total memory usage in bytes.
    #[inline]
    #[must_use]
    pub const fn memory_bytes(&self) -> usize {
        self.capacity * self.dimension * std::mem::size_of::<f32>()
    }

    /// Inserts a vector at a specific index.
    ///
    /// Automatically grows capacity if needed.
    /// Note: This allows sparse population. Uninitialized slots contain undefined
    /// data (or 0.0 if alloc gave zeroed memory).
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::Error::DimensionMismatch`] if `vector.len() != self.dimension`.
    /// Returns [`Error::AllocationFailed`] if capacity growth fails.
    ///
    /// [`crate::error::Error::DimensionMismatch`]: crate::error::Error::DimensionMismatch
    /// [`Error::AllocationFailed`]: crate::error::Error::AllocationFailed
    pub fn insert_at(&mut self, index: usize, vector: &[f32]) -> crate::error::Result<()> {
        validate_dimension_match(self.dimension, vector.len())?;

        self.ensure_capacity(index + 1)?;

        let offset = index * self.dimension;
        // SAFETY: We ensured capacity covers index, data is non-null (NonNull invariant)
        // - Condition 1: Capacity was verified to cover the target index.
        // - Condition 2: Both source and destination pointers are valid and properly aligned.
        // SAFETY: Efficient bulk memory copy for vector insertion.
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
        Ok(())
    }

    /// Adds a vector to the storage.
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::Error::DimensionMismatch`] if `vector.len() != self.dimension`.
    /// Returns [`Error::AllocationFailed`] if capacity growth fails.
    ///
    /// [`crate::error::Error::DimensionMismatch`]: crate::error::Error::DimensionMismatch
    /// [`Error::AllocationFailed`]: crate::error::Error::AllocationFailed
    pub fn push(&mut self, vector: &[f32]) -> crate::error::Result<()> {
        self.insert_at(self.count, vector)
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
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::Error::DimensionMismatch`] or [`Error::AllocationFailed`] on the
    /// first vector that fails. Vectors added before the failure remain in storage.
    ///
    /// [`crate::error::Error::DimensionMismatch`]: crate::error::Error::DimensionMismatch
    /// [`Error::AllocationFailed`]: crate::error::Error::AllocationFailed
    pub fn push_batch(&mut self, vectors: &[&[f32]]) -> crate::error::Result<usize> {
        if vectors.is_empty() {
            return Ok(0);
        }
        // Validate all dimensions upfront to prevent partial writes on error.
        for vector in vectors {
            validate_dimension_match(self.dimension, vector.len())?;
        }
        self.ensure_capacity(self.count + vectors.len())?;
        for vector in vectors {
            let offset = self.count * self.dimension;
            // SAFETY: ensure_capacity (called above) guarantees room for
            // self.count + vectors.len() elements, and all dimensions were
            // validated above so offset + dimension is within bounds.
            // - Condition 1: offset + dimension is within allocated buffer.
            // - Condition 2: Both pointers are valid and aligned for f32.
            // - Condition 3: &mut self guarantees exclusive access — no data race.
            // SAFETY: Batch push with single pre-allocation.
            unsafe {
                std::ptr::copy_nonoverlapping(
                    vector.as_ptr(),
                    self.data.as_ptr().add(offset),
                    self.dimension,
                );
            }
            self.count += 1;
        }
        Ok(vectors.len())
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
        // - Condition 1: index < count ensures access is within initialized range.
        // - Condition 2: data is non-null per NonNull invariant.
        // SAFETY: Zero-copy slice creation from contiguous storage.
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
        // - Condition 1: Caller contract ensures index < count.
        // - Condition 2: data is non-null per NonNull invariant.
        // SAFETY: Performance-critical path requiring unchecked access.
        std::slice::from_raw_parts(self.data.as_ptr().add(offset), self.dimension)
    }

    /// Prefetches a vector into multiple cache levels for upcoming access.
    ///
    /// Uses cross-platform multi-cache-line prefetch (`x86_64` + `aarch64` + no-op fallback)
    /// to warm CPU caches before SIMD distance computation.
    #[inline]
    pub fn prefetch(&self, index: usize) {
        if index < self.count {
            let offset = index * self.dimension;
            // SAFETY: index < count implies offset is within allocated range,
            // data is non-null per NonNull invariant.
            // - Condition 1: Bounds check ensures offset + dimension <= capacity * dimension.
            // - Condition 2: NonNull guarantees pointer validity.
            // SAFETY: Create slice for cross-platform multi-cache-line prefetch.
            let vector = unsafe {
                std::slice::from_raw_parts(self.data.as_ptr().add(offset), self.dimension)
            };
            crate::simd_native::prefetch_vector_multi_cache_line(vector);
        }
    }
}

// Backward-compatible re-exports from contiguous_ops
pub use crate::contiguous_ops::{
    batch_cosine_similarities, batch_dot_products_simd, pad_to_simd_width,
};
