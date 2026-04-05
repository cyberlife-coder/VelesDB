//! Reorder, batch distance, and lifecycle operations for `ContiguousVectors`.
//!
//! Extracted from `perf_optimizations.rs` to reduce NLOC.
//! Contains reorder permutation, dot-product batching, Drop, and free SIMD helpers.

use super::perf_optimizations::ContiguousVectors;
use std::alloc::dealloc;
use std::ptr::{self, NonNull};

// =============================================================================
// ContiguousVectors: Reorder + Dot-Product + Drop
// =============================================================================

impl ContiguousVectors {
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
// Batch Distance Computation (free functions)
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
/// use velesdb_core::contiguous_ops::pad_to_simd_width;
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
