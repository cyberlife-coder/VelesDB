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

use std::alloc::{alloc_zeroed, dealloc, Layout};
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

// SAFETY: `ContiguousVectors` is `Send` because it owns its allocation.
// - Condition 1: The backing buffer is uniquely owned by the struct.
// - Condition 2: Mutation requires `&mut self` or lock-guarded interior access.
// Reason: Moving ownership of this container between threads is sound.
unsafe impl Send for ContiguousVectors {}
// SAFETY: `ContiguousVectors` is `Sync` because shared access is read-only.
// - Condition 1: All writes happen through methods requiring mutable or exclusive lock access.
// - Condition 2: Returned shared slices borrow immutably and cannot mutate internal state.
// Reason: Concurrent shared references cannot violate aliasing rules.
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
        // Reason: Zero-initialized allocation guarantees all f32 slots are 0.0,
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
    fn layout(dimension: usize, capacity: usize) -> crate::error::Result<Layout> {
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
        // Reason: Zero-copy GPU upload requires a contiguous &[f32] view.
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

    /// Ensures the storage has capacity for at least `required_capacity` vectors.
    ///
    /// # Errors
    ///
    /// Returns [`Error::AllocationFailed`] if reallocation fails.
    ///
    /// [`Error::AllocationFailed`]: crate::error::Error::AllocationFailed
    pub fn ensure_capacity(&mut self, required_capacity: usize) -> crate::error::Result<()> {
        if required_capacity > self.capacity {
            let new_capacity = required_capacity.max(self.capacity * 2);
            self.resize(new_capacity)?;
        }
        Ok(())
    }

    /// Inserts a vector at a specific index.
    ///
    /// Automatically grows capacity if needed.
    /// Note: This allows sparse population. Uninitialized slots contain undefined
    /// data (or 0.0 if alloc gave zeroed memory).
    ///
    /// # Errors
    ///
    /// Returns [`Error::DimensionMismatch`] if `vector.len() != self.dimension`.
    /// Returns [`Error::AllocationFailed`] if capacity growth fails.
    ///
    /// [`Error::DimensionMismatch`]: crate::error::Error::DimensionMismatch
    /// [`Error::AllocationFailed`]: crate::error::Error::AllocationFailed
    pub fn insert_at(&mut self, index: usize, vector: &[f32]) -> crate::error::Result<()> {
        if vector.len() != self.dimension {
            return Err(crate::error::Error::DimensionMismatch {
                expected: self.dimension,
                actual: vector.len(),
            });
        }

        self.ensure_capacity(index + 1)?;

        let offset = index * self.dimension;
        // SAFETY: We ensured capacity covers index, data is non-null (NonNull invariant)
        // - Condition 1: Capacity was verified to cover the target index.
        // - Condition 2: Both source and destination pointers are valid and properly aligned.
        // Reason: Efficient bulk memory copy for vector insertion.
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
    /// Returns [`Error::DimensionMismatch`] if `vector.len() != self.dimension`.
    /// Returns [`Error::AllocationFailed`] if capacity growth fails.
    ///
    /// [`Error::DimensionMismatch`]: crate::error::Error::DimensionMismatch
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
    /// Returns [`Error::DimensionMismatch`] or [`Error::AllocationFailed`] on the
    /// first vector that fails. Vectors added before the failure remain in storage.
    ///
    /// [`Error::DimensionMismatch`]: crate::error::Error::DimensionMismatch
    /// [`Error::AllocationFailed`]: crate::error::Error::AllocationFailed
    pub fn push_batch<'a>(
        &mut self,
        vectors: impl Iterator<Item = &'a [f32]>,
    ) -> crate::error::Result<usize> {
        let mut added = 0;
        for vector in vectors {
            self.push(vector)?;
            added += 1;
        }
        Ok(added)
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
        // Reason: Zero-copy slice creation from contiguous storage.
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
        // Reason: Performance-critical path requiring unchecked access.
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
            // Reason: Create slice for cross-platform multi-cache-line prefetch.
            let vector = unsafe {
                std::slice::from_raw_parts(self.data.as_ptr().add(offset), self.dimension)
            };
            crate::simd_native::prefetch_vector_multi_cache_line(vector);
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
    fn resize(&mut self, new_capacity: usize) -> crate::error::Result<()> {
        if new_capacity <= self.capacity {
            return Ok(());
        }

        let old_layout = Self::layout(self.dimension, self.capacity)?;
        let new_layout = Self::layout(self.dimension, new_capacity)?;

        let new_data = Self::alloc_and_copy(new_layout, self.data, self.count, self.dimension)?;

        // Deallocate old buffer
        // SAFETY: self.data was allocated with old_layout, is non-null (NonNull invariant)
        // - Condition 1: old_layout matches the allocation parameters.
        // - Condition 2: Pointer is non-null per NonNull invariant.
        // Reason: Free old buffer after data migration to new buffer.
        unsafe {
            dealloc(self.data.as_ptr().cast::<u8>(), old_layout);
        }

        // Update state (all-or-nothing)
        self.data = new_data;
        self.capacity = new_capacity;
        Ok(())
    }

    /// Allocates a new buffer and copies existing data into it.
    ///
    /// Uses `AllocGuard` for panic-safety: if copy panics, the guard drops
    /// and frees the new buffer automatically.
    #[allow(clippy::cast_ptr_alignment)] // Layout is 64-byte aligned
    fn alloc_and_copy(
        new_layout: Layout,
        src: NonNull<f32>,
        count: usize,
        dimension: usize,
    ) -> crate::error::Result<NonNull<f32>> {
        use crate::alloc_guard::AllocGuard;

        // Allocate zero-initialized buffer with RAII guard (PERF-002)
        let guard = AllocGuard::new_zeroed(new_layout).ok_or_else(|| {
            crate::error::Error::AllocationFailed(format!(
                "Failed to allocate {} bytes for ContiguousVectors resize",
                new_layout.size()
            ))
        })?;

        // EPIC-032/US-002: Use NonNull for type-level guarantee
        let new_data = NonNull::new(guard.cast::<f32>()).ok_or_else(|| {
            crate::error::Error::AllocationFailed("AllocGuard returned null pointer".to_string())
        })?;

        // Copy existing data to new buffer
        if count > 0 {
            let copy_size = count * dimension;
            // SAFETY: Both pointers are valid (NonNull), non-overlapping, and properly aligned
            // - Condition 1: Source pointer (src) is valid and properly aligned.
            // - Condition 2: Destination pointer (new_data) is valid and properly aligned.
            // - Condition 3: Pointers are non-overlapping (old and new allocations are distinct).
            // Reason: Migrate data to newly allocated buffer during resize.
            unsafe {
                ptr::copy_nonoverlapping(src.as_ptr(), new_data.as_ptr(), copy_size);
            }
        }

        // Transfer ownership - guard won't free on drop anymore
        let _ = guard.into_raw();

        Ok(new_data)
    }

    /// Reorders vectors according to the given permutation.
    ///
    /// `new_order[i]` contains the old index of the vector that should occupy
    /// position `i` after reordering. The permutation must have exactly
    /// `self.len()` elements and every index must be `< self.len()`.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - `new_order.len() != self.len()`
    /// - Any index in `new_order` is out of bounds
    /// - The new buffer allocation fails
    pub fn reorder(&mut self, new_order: &[usize]) -> crate::error::Result<()> {
        if new_order.len() != self.count {
            return Err(crate::error::Error::Internal(format!(
                "Reorder permutation length {} != vector count {}",
                new_order.len(),
                self.count
            )));
        }
        if self.count == 0 {
            return Ok(());
        }

        self.reorder_copy(new_order)
    }

    /// Performs the out-of-place vector copy for reordering.
    ///
    /// Allocates a temporary buffer, copies vectors in permuted order, then
    /// swaps the buffer into place. Uses `AllocGuard` for panic-safety.
    fn reorder_copy(&mut self, new_order: &[usize]) -> crate::error::Result<()> {
        use crate::alloc_guard::AllocGuard;

        let new_layout = Self::layout(self.dimension, self.count)?;
        let guard = AllocGuard::new_zeroed(new_layout).ok_or_else(|| {
            crate::error::Error::AllocationFailed(format!(
                "Reorder: failed to allocate {} bytes",
                new_layout.size()
            ))
        })?;
        let new_ptr = NonNull::new(guard.cast::<f32>()).ok_or_else(|| {
            crate::error::Error::AllocationFailed(
                "Reorder: AllocGuard returned null pointer".to_string(),
            )
        })?;

        self.copy_permuted_vectors(new_ptr.as_ptr(), new_order)?;

        // Transfer ownership — guard will not free on drop
        let _ = guard.into_raw();

        // Deallocate old buffer
        let old_layout = Self::layout(self.dimension, self.capacity)?;
        // SAFETY: self.data was allocated with old_layout, is non-null (NonNull invariant).
        // - Condition 1: old_layout matches the allocation parameters.
        // - Condition 2: Pointer is non-null per NonNull invariant.
        // Reason: Free old buffer after data migration to reordered buffer.
        unsafe { dealloc(self.data.as_ptr().cast::<u8>(), old_layout) };

        self.data = new_ptr;
        self.capacity = self.count;
        Ok(())
    }

    /// Copies vectors from the current buffer to `dst` in permuted order.
    fn copy_permuted_vectors(
        &self,
        dst: *mut f32,
        new_order: &[usize],
    ) -> crate::error::Result<()> {
        let dim = self.dimension;
        for (new_idx, &old_idx) in new_order.iter().enumerate() {
            if old_idx >= self.count {
                return Err(crate::error::Error::Internal(format!(
                    "Reorder index {old_idx} out of bounds (count={})",
                    self.count
                )));
            }
            // SAFETY: src is within the current allocation (old_idx < count, count <= capacity).
            // dst is within the new allocation (new_idx < new_order.len() == count).
            // Both buffers are distinct (non-overlapping) allocations with room for `dim` f32s.
            // - Condition 1: old_idx < count ensures src offset is in bounds.
            // - Condition 2: new_idx < count ensures dst offset is in bounds.
            // Reason: Out-of-place copy for cache-locality reordering.
            unsafe {
                ptr::copy_nonoverlapping(
                    self.data.as_ptr().add(old_idx * dim),
                    dst.add(new_idx * dim),
                    dim,
                );
            }
        }
        Ok(())
    }

    /// Computes dot product with another vector using SIMD.
    #[inline]
    #[must_use]
    pub fn dot_product(&self, index: usize, query: &[f32]) -> Option<f32> {
        let vector = self.get(index)?;
        Some(crate::simd_native::dot_product_native(vector, query))
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
        // Layout was valid at construction; it must still be valid at drop.
        let Ok(layout) = Self::layout(self.dimension, self.capacity) else {
            // Layout was valid at construction; this branch is unreachable
            // unless memory corruption occurred. Leak memory rather than abort.
            tracing::error!(
                "ContiguousVectors::drop: layout computation failed \
                 (dim={}, cap={}), leaking memory",
                self.dimension,
                self.capacity,
            );
            return;
        };
        // SAFETY: data was allocated with this layout, is non-null (NonNull invariant)
        // - Condition 1: Layout matches original allocation parameters.
        // - Condition 2: Pointer is non-null per NonNull invariant.
        // Reason: Release allocated memory when ContiguousVectors is dropped.
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
/// F-17: Delegates to `batch_dot_product_native` which includes `x86_64`
/// prefetch hints for upcoming candidate vectors.
#[must_use]
pub fn batch_dot_products_simd(vectors: &[&[f32]], query: &[f32]) -> Vec<f32> {
    crate::simd_native::batch_dot_product_native(vectors, query)
}

// =============================================================================
// SIMD Padding Utility
// =============================================================================

/// AVX2 register width for `f32` lanes: 256 bits / 32 bits = 8 lanes.
const SIMD_WIDTH: usize = 8;

/// Pads a vector to the next multiple of 8 (AVX2 register width for `f32`).
///
/// Appending zeros does not affect distance computations (cosine, euclidean, dot)
/// when the query and stored vectors share the same padded length.
///
/// Returns an empty `Vec` when the input is empty (0 is already a multiple of 8).
///
/// # Examples
///
/// ```
/// use velesdb_core::perf_optimizations::pad_to_simd_width;
///
/// let v = vec![1.0_f32, 2.0, 3.0];
/// let padded = pad_to_simd_width(&v);
/// assert_eq!(padded.len(), 8);
/// assert_eq!(&padded[..3], &[1.0, 2.0, 3.0]);
/// ```
#[must_use]
pub fn pad_to_simd_width(vector: &[f32]) -> Vec<f32> {
    let len = vector.len();
    if len == 0 {
        return Vec::new();
    }
    let padded_len = len.div_ceil(SIMD_WIDTH) * SIMD_WIDTH;
    let mut padded = vec![0.0_f32; padded_len];
    padded[..len].copy_from_slice(vector);
    padded
}

/// Computes multiple cosine similarities in a single pass with prefetch.
#[must_use]
pub fn batch_cosine_similarities(vectors: &[&[f32]], query: &[f32]) -> Vec<f32> {
    let prefetch_distance = crate::simd_native::calculate_prefetch_distance(query.len());
    let mut results = Vec::with_capacity(vectors.len());

    for (i, v) in vectors.iter().enumerate() {
        if i + prefetch_distance < vectors.len() {
            crate::simd_native::prefetch_vector(vectors[i + prefetch_distance]);
        }
        results.push(crate::simd_native::cosine_similarity_native(v, query));
    }

    results
}
