// Reason: PDX block-columnar storage — fields and accessors used by columnar_distance
// kernels and tests. Production wiring via reorder_for_locality populates the struct.
#![allow(dead_code)]

//! Block-columnar (PDX) vector storage for SIMD-parallel distance computation.
//!
//! # Memory Layout
//!
//! Standard Array-of-Structures (AoS) stores vectors contiguously:
//! ```text
//! [v0_d0, v0_d1, ..., v0_dD, v1_d0, v1_d1, ..., v1_dD, ...]
//! ```
//!
//! PDX block-columnar layout groups `B` vectors into blocks, with dimensions
//! interleaved within each block:
//! ```text
//! Block k: [v_{kB}_d0,   ..., v_{kB+B-1}_d0,    // dim 0
//!           v_{kB}_d1,   ..., v_{kB+B-1}_d1,    // dim 1
//!           ...
//!           v_{kB}_dD-1, ..., v_{kB+B-1}_dD-1]  // dim D-1
//! ```
//!
//! This enables broadcasting `query[d]` once per dimension and computing the
//! d-th contribution for all `B` vectors simultaneously, achieving 64x better
//! register reuse vs AoS where `query[d]` is loaded once per vector.
//!
//! # References
//!
//! Pirk, H. et al. "Efficient Cross-Columnar Sorting" (PDX layout).

use crate::perf_optimizations::ContiguousVectors;

/// Number of vectors per PDX block.
///
/// Chosen as 64 to align with AVX-512 (16 f32 lanes) and AVX2 (8 f32 lanes).
/// The inner loop over 64 elements auto-vectorizes to 4 AVX-512 iterations
/// or 8 AVX2 iterations per dimension.
pub(crate) const PDX_BLOCK_SIZE: usize = 64;

/// Block-columnar vector storage for SIMD-parallel distance computation.
///
/// Stores vectors in PDX layout: within each block of 64 vectors, dimensions
/// are interleaved so that a single dimension's values for all block vectors
/// are contiguous in memory.
///
/// # Usage
///
/// This is an offline/batch structure. Convert from [`ContiguousVectors`] via
/// [`from_contiguous`](ColumnarVectors::from_contiguous), then use the block
/// distance kernels in [`columnar_distance`](super::columnar_distance).
#[derive(Debug)]
pub(crate) struct ColumnarVectors {
    /// Block-columnar buffer. Each block occupies `PDX_BLOCK_SIZE * dimension`
    /// f32 slots, even the last block (zero-padded for partial occupancy).
    data: Vec<f32>,
    /// Vector dimensionality.
    dimension: usize,
    /// Total number of vectors stored (including partial last block).
    count: usize,
    /// Number of blocks (ceil(count / PDX_BLOCK_SIZE)).
    num_blocks: usize,
}

impl ColumnarVectors {
    /// Transposes AoS vectors from a [`ContiguousVectors`] into PDX layout.
    ///
    /// The last block is zero-padded if `vectors.len()` is not a multiple of
    /// [`PDX_BLOCK_SIZE`]. Zero-padding is safe for distance computation:
    /// squared-L2 and dot-product contributions from zero-padded slots are
    /// zeroed out by the caller via the `block_size` parameter.
    #[must_use]
    pub(crate) fn from_contiguous(vectors: &ContiguousVectors) -> Self {
        let count = vectors.len();
        let dimension = vectors.dimension();

        if count == 0 {
            return Self {
                data: Vec::new(),
                dimension,
                count: 0,
                num_blocks: 0,
            };
        }

        let num_blocks = count.div_ceil(PDX_BLOCK_SIZE);
        let total = num_blocks * PDX_BLOCK_SIZE * dimension;
        let mut data = vec![0.0_f32; total];

        transpose_aos_to_pdx(vectors, &mut data, dimension, num_blocks);

        Self {
            data,
            dimension,
            count,
            num_blocks,
        }
    }

    /// Returns the number of PDX blocks.
    #[inline]
    #[must_use]
    pub(crate) fn block_count(&self) -> usize {
        self.num_blocks
    }

    /// Returns the number of valid vectors in the given block.
    ///
    /// All blocks except possibly the last contain [`PDX_BLOCK_SIZE`] vectors.
    /// The last block contains `count % PDX_BLOCK_SIZE` vectors (or
    /// `PDX_BLOCK_SIZE` if count is an exact multiple).
    #[inline]
    #[must_use]
    pub(crate) fn block_size(&self, block_idx: usize) -> usize {
        debug_assert!(block_idx < self.num_blocks, "block index out of bounds");
        if block_idx + 1 < self.num_blocks {
            PDX_BLOCK_SIZE
        } else {
            let remainder = self.count % PDX_BLOCK_SIZE;
            if remainder == 0 {
                PDX_BLOCK_SIZE
            } else {
                remainder
            }
        }
    }

    /// Returns the PDX data slice for an entire block.
    ///
    /// The slice has length `PDX_BLOCK_SIZE * dimension` (always full block
    /// size, zero-padded for partial blocks).
    #[inline]
    #[must_use]
    pub(crate) fn block_ptr(&self, block_idx: usize) -> &[f32] {
        debug_assert!(block_idx < self.num_blocks, "block index out of bounds");
        let block_len = PDX_BLOCK_SIZE * self.dimension;
        let start = block_idx * block_len;
        &self.data[start..start + block_len]
    }

    /// Returns the vector dimensionality.
    #[inline]
    #[must_use]
    pub(crate) fn dimension(&self) -> usize {
        self.dimension
    }

    /// Returns the total number of vectors stored.
    #[inline]
    #[must_use]
    pub(crate) fn len(&self) -> usize {
        self.count
    }

    /// Returns `true` if no vectors are stored.
    #[inline]
    #[must_use]
    pub(crate) fn is_empty(&self) -> bool {
        self.count == 0
    }
}

/// Transpose AoS data from `ContiguousVectors` into PDX block-columnar layout.
///
/// For each block, each dimension, and each vector within the block:
/// ```text
/// pdx[block_offset + d * PDX_BLOCK_SIZE + local] = aos[vec_idx * dim + d]
/// ```
fn transpose_aos_to_pdx(
    vectors: &ContiguousVectors,
    pdx: &mut [f32],
    dimension: usize,
    num_blocks: usize,
) {
    let block_stride = PDX_BLOCK_SIZE * dimension;
    let flat = vectors.as_flat_slice();

    for block_idx in 0..num_blocks {
        let block_offset = block_idx * block_stride;
        let base_vec = block_idx * PDX_BLOCK_SIZE;
        let block_count = vectors.len().saturating_sub(base_vec).min(PDX_BLOCK_SIZE);

        for d in 0..dimension {
            let dim_offset = block_offset + d * PDX_BLOCK_SIZE;
            for local in 0..block_count {
                let vec_idx = base_vec + local;
                pdx[dim_offset + local] = flat[vec_idx * dimension + d];
            }
            // Remaining slots (local in block_count..PDX_BLOCK_SIZE) are
            // already zero from vec![0.0; total] initialization.
        }
    }
}
