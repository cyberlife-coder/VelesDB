//! Reorder, batch distance, and lifecycle operations for `ContiguousVectors`.
//!
//! Extracted from `perf_optimizations.rs` to reduce NLOC.
//! Contains reorder permutation, dot-product batching, Drop, and free SIMD helpers.

use super::perf_optimizations::ContiguousVectors;
use std::alloc::dealloc;

// =============================================================================
// ContiguousVectors: Reorder + Dot-Product + Drop
// =============================================================================

impl ContiguousVectors {
    /// Reorders vectors according to the given permutation.
    ///
    /// `new_order[i]` contains the old index of the vector that should occupy
    /// position `i` after reordering. It must be a genuine permutation of
    /// `0..self.len()`: exactly `self.len()` entries, each index appearing
    /// exactly once.
    ///
    /// # The backing survives
    ///
    /// The permutation is applied **in place**, through whichever buffer the
    /// arena already holds, so a file-mapped arena is still file-mapped when
    /// this returns (#2112). Its pages stay evictable — the property the
    /// mapped backing exists for — [`flush_backing`](Self::flush_backing)
    /// still reaches the file, and the bytes that land on disk are the
    /// reordered ones. The predecessor of this implementation gathered into a
    /// fresh heap buffer and silently demoted the arena, which cost the
    /// eviction saving and turned `flush_backing` into a no-op returning
    /// `Ok`.
    ///
    /// In place is also what the heap arena wants: no second buffer means the
    /// peak allocation during a reorder is the arena itself rather than twice
    /// it. `capacity` is therefore left alone, where the copying version
    /// shrank it to `count` and made the next `push` reallocate.
    ///
    /// # Cost
    ///
    /// Each vector is written once, plus one scratch copy per permutation
    /// cycle; the bookkeeping is a `bool` per vector and one vector of
    /// scratch — 100 KiB and 3 KiB respectively at 100 000 × 768-d, against
    /// the 293 MiB second arena the copy needed.
    ///
    /// Measured at 100 000 × 768-d over a single cycle covering every vector,
    /// which is the unfriendly case (2026-08-24, 4-vCPU Linux container,
    /// eight runs): **0.044–0.060 s in place against 0.158–0.209 s** for the
    /// same scattered reads gathered into a fresh buffer, and the two
    /// backings are within noise of each other. Ranges rather than points
    /// because a single run of either understates the spread on a shared
    /// host. Reproduce with the `reorder` mode of the `resident_set` example.
    ///
    /// Those figures are with the arena resident, which is the state a
    /// post-build reorder finds it in. On pages the kernel has already
    /// reclaimed, in place faults them back — but so does a copy, which has
    /// to read every vector too, so the ordering does not change.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - `new_order.len() != self.len()`
    /// - Any index in `new_order` is out of bounds
    /// - Any index appears more than once — a non-bijective `new_order` would
    ///   duplicate vectors and lose others, which the copying predecessor did
    ///   silently
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
        validate_permutation(new_order, self.count)?;

        let dimension = self.dimension;
        permute_in_place(self.live_flat_mut(), dimension, new_order);
        Ok(())
    }

    /// The `count * dimension` live f32s as one mutable slice.
    ///
    /// Private to this module: it hands out write access to the whole arena
    /// at once, which only the permutation needs. Everything else goes
    /// through `get_mut`, whose bounds check is the point.
    fn live_flat_mut(&mut self) -> &mut [f32] {
        // `count <= capacity` and `capacity * dimension` was validated to fit
        // in `usize` at allocation time, so this product cannot overflow.
        let total = self.count.saturating_mul(self.dimension);
        // SAFETY: `data` addresses `capacity * dimension` initialized f32s —
        // both the heap allocation (`alloc_zeroed`) and the mapped arena
        // (a file zero-extended to its declared length) start zeroed.
        // - Condition 1: `data` is a valid, aligned `NonNull<f32>` pointer,
        //   for the mapped backing because `DATA_OFFSET` is a whole number of
        //   pages and the mmap base is page-aligned.
        // - Condition 2: `total <= capacity * dimension`, so the slice stays
        //   inside the allocation.
        // - Condition 3: `&mut self` excludes any other live reference to
        //   these bytes for the slice's lifetime.
        // SAFETY: The in-place permutation needs one mutable view of the arena.
        unsafe { std::slice::from_raw_parts_mut(self.data.as_ptr(), total) }
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
        // A file-mapped arena's bytes belong to the mapping, not the
        // allocator: releasing them is dropping `backing`, which happens on
        // its own after this. Handing that pointer to `dealloc` would be
        // undefined behaviour, so the backing is checked before anything else
        // in this function touches the allocator (#2112).
        if !self.backing.is_heap() {
            return;
        }
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
        // SAFETY: Release allocated memory when ContiguousVectors is dropped.
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

/// Rejects anything that is not a bijection of `0..count`.
///
/// Checked before the permutation rather than during it: the in-place
/// algorithm follows cycles, and a repeated index makes a cycle that never
/// closes. Validating first turns that into an error instead of a hang, and
/// also catches the duplicate-vector case the copying implementation used to
/// accept silently.
fn validate_permutation(new_order: &[usize], count: usize) -> crate::error::Result<()> {
    let mut seen = vec![false; count];
    for &old_idx in new_order {
        let slot = seen.get_mut(old_idx).ok_or_else(|| {
            crate::error::Error::Internal(format!(
                "Reorder index {old_idx} out of bounds (count={count})"
            ))
        })?;
        if *slot {
            return Err(crate::error::Error::Internal(format!(
                "Reorder index {old_idx} appears twice; new_order must be a permutation"
            )));
        }
        *slot = true;
    }
    Ok(())
}

/// Applies `new_order` to `flat` without a second arena.
///
/// A permutation decomposes into disjoint cycles, and each cycle can be
/// rotated with a single vector of scratch space. `moved` is what keeps the
/// outer loop from re-entering a cycle it already rotated.
///
/// `new_order` is a validated bijection of `0..new_order.len()` and `flat`
/// holds `new_order.len() * dimension` f32s, so every index computed here is
/// in bounds.
fn permute_in_place(flat: &mut [f32], dimension: usize, new_order: &[usize]) {
    let mut scratch = vec![0.0_f32; dimension];
    let mut moved = vec![false; new_order.len()];

    for start in 0..new_order.len() {
        if moved[start] {
            continue;
        }
        // A fixed point is its own cycle; rotating it would copy a vector
        // out to scratch and straight back.
        if new_order[start] == start {
            moved[start] = true;
            continue;
        }
        rotate_cycle(flat, dimension, new_order, start, &mut moved, &mut scratch);
    }
}

/// Rotates the one permutation cycle that contains `start`.
///
/// `flat[start]` is parked in `scratch` first, which frees the slot the cycle
/// needs; every other member is then pulled from its source, whose own move
/// has not happened yet. The vector in scratch closes the cycle.
fn rotate_cycle(
    flat: &mut [f32],
    dimension: usize,
    new_order: &[usize],
    start: usize,
    moved: &mut [bool],
    scratch: &mut [f32],
) {
    scratch.copy_from_slice(&flat[start * dimension..][..dimension]);
    let mut dst = start;
    loop {
        moved[dst] = true;
        let src = new_order[dst];
        if src == start {
            flat[dst * dimension..][..dimension].copy_from_slice(scratch);
            return;
        }
        flat.copy_within(
            src * dimension..src * dimension + dimension,
            dst * dimension,
        );
        dst = src;
    }
}
