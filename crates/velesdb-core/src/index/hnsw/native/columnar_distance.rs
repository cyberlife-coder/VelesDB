//! Block-parallel distance kernels for PDX columnar layout.
//!
//! Each kernel computes distances from a single query vector to ALL vectors
//! in one PDX block simultaneously. The key optimization: `query[d]` is
//! broadcast once per dimension, then applied to all 64 vectors in the block.
//!
//! # Auto-Vectorization
//!
//! These kernels use scalar Rust with fixed-size arrays. LLVM auto-vectorizes
//! the inner loop over `PDX_BLOCK_SIZE` (64) elements into:
//! - 4 iterations with AVX-512 (16 f32 lanes)
//! - 8 iterations with AVX2 (8 f32 lanes)
//! - 16 iterations with NEON (4 f32 lanes)
//!
//! No manual SIMD intrinsics are needed.

use super::columnar_vectors::PDX_BLOCK_SIZE;

/// Computes squared L2 distances from `query` to all vectors in a PDX block.
///
/// # Arguments
///
/// * `query` - Query vector of length `dimension`.
/// * `block` - PDX block data (length `PDX_BLOCK_SIZE * dimension`).
/// * `dimension` - Vector dimensionality.
/// * `block_size` - Actual vector count in this block (1..=64).
///
/// # Returns
///
/// Array of 64 distances. Only indices `0..block_size` contain valid results;
/// remaining slots are unspecified.
///
/// # Algorithm
///
/// ```text
/// for d in 0..dimension:
///     q_d = query[d]
///     for v in 0..64:
///         diff = q_d - block[d * 64 + v]
///         acc[v] += diff * diff
/// ```
#[must_use]
pub(crate) fn block_squared_l2(
    query: &[f32],
    block: &[f32],
    dimension: usize,
    block_size: usize,
) -> [f32; PDX_BLOCK_SIZE] {
    debug_assert_eq!(query.len(), dimension);
    debug_assert_eq!(block.len(), PDX_BLOCK_SIZE * dimension);
    debug_assert!(block_size <= PDX_BLOCK_SIZE);

    let mut acc = [0.0_f32; PDX_BLOCK_SIZE];
    accumulate_squared_diff(&mut acc, query, block, dimension);
    zero_padding_slots(&mut acc, block_size);
    acc
}

/// Computes negative dot products from `query` to all vectors in a PDX block.
///
/// Returns negative dot products so that lower values indicate higher
/// similarity, consistent with distance-based nearest-neighbor search.
///
/// # Arguments
///
/// * `query` - Query vector of length `dimension`.
/// * `block` - PDX block data (length `PDX_BLOCK_SIZE * dimension`).
/// * `dimension` - Vector dimensionality.
/// * `block_size` - Actual vector count in this block (1..=64).
///
/// # Returns
///
/// Array of 64 negative dot products. Only indices `0..block_size` are valid.
#[must_use]
pub(crate) fn block_dot_product(
    query: &[f32],
    block: &[f32],
    dimension: usize,
    block_size: usize,
) -> [f32; PDX_BLOCK_SIZE] {
    debug_assert_eq!(query.len(), dimension);
    debug_assert_eq!(block.len(), PDX_BLOCK_SIZE * dimension);
    debug_assert!(block_size <= PDX_BLOCK_SIZE);

    let mut acc = [0.0_f32; PDX_BLOCK_SIZE];
    accumulate_products(&mut acc, query, block, dimension);
    negate_and_zero_padding(&mut acc, block_size);
    acc
}

/// Computes cosine distances from `query` to all vectors in a PDX block.
///
/// Returns `1.0 - cosine_similarity` so that lower values indicate higher
/// similarity, consistent with distance-based nearest-neighbor search.
///
/// # Arguments
///
/// * `query` - Query vector of length `dimension`.
/// * `block` - PDX block data (length `PDX_BLOCK_SIZE * dimension`).
/// * `dimension` - Vector dimensionality.
/// * `block_size` - Actual vector count in this block (1..=64).
///
/// # Returns
///
/// Array of 64 cosine distances. Only indices `0..block_size` are valid.
#[must_use]
pub(crate) fn block_cosine_distance(
    query: &[f32],
    block: &[f32],
    dimension: usize,
    block_size: usize,
) -> [f32; PDX_BLOCK_SIZE] {
    debug_assert_eq!(query.len(), dimension);
    debug_assert_eq!(block.len(), PDX_BLOCK_SIZE * dimension);
    debug_assert!(block_size <= PDX_BLOCK_SIZE);

    let (dot, norm_b_sq) = accumulate_dot_and_norm(query, block, dimension);
    let query_norm_sq = query_norm_squared(query);
    finalize_cosine_distances(dot, norm_b_sq, query_norm_sq, block_size)
}

// ---------------------------------------------------------------------------
// Internal helpers (extracted to keep public functions under CC=8 / NLOC=50)
// ---------------------------------------------------------------------------

/// Accumulates squared differences for all dimensions into `acc`.
///
/// Inner loop iterates over `PDX_BLOCK_SIZE` contiguous f32 values per
/// dimension, enabling LLVM auto-vectorization.
// Reason: range indexing is required for LLVM auto-vectorization of [f32; 64]
#[allow(clippy::needless_range_loop)]
#[inline]
fn accumulate_squared_diff(
    acc: &mut [f32; PDX_BLOCK_SIZE],
    query: &[f32],
    block: &[f32],
    dimension: usize,
) {
    for d in 0..dimension {
        let q_d = query[d];
        let base = d * PDX_BLOCK_SIZE;
        for v in 0..PDX_BLOCK_SIZE {
            let diff = q_d - block[base + v];
            acc[v] += diff * diff;
        }
    }
}

/// Accumulates dot products for all dimensions into `acc`.
// Reason: range indexing is required for LLVM auto-vectorization of [f32; 64]
#[allow(clippy::needless_range_loop)]
#[inline]
fn accumulate_products(
    acc: &mut [f32; PDX_BLOCK_SIZE],
    query: &[f32],
    block: &[f32],
    dimension: usize,
) {
    for d in 0..dimension {
        let q_d = query[d];
        let base = d * PDX_BLOCK_SIZE;
        for v in 0..PDX_BLOCK_SIZE {
            acc[v] += q_d * block[base + v];
        }
    }
}

/// Negates dot products and zeros padding slots beyond `block_size`.
#[inline]
fn negate_and_zero_padding(acc: &mut [f32; PDX_BLOCK_SIZE], block_size: usize) {
    for item in acc.iter_mut().take(block_size) {
        *item = -*item;
    }
    for item in acc.iter_mut().skip(block_size) {
        *item = 0.0;
    }
}

/// Zeros padding slots beyond `block_size`.
#[inline]
fn zero_padding_slots(acc: &mut [f32; PDX_BLOCK_SIZE], block_size: usize) {
    for item in acc.iter_mut().skip(block_size) {
        *item = 0.0;
    }
}

/// Accumulates dot products and norm-B-squared in a fused single pass.
///
/// Returns `(dot[64], norm_b_sq[64])`.
// Reason: range indexing is required for LLVM auto-vectorization of [f32; 64]
#[allow(clippy::needless_range_loop)]
#[inline]
fn accumulate_dot_and_norm(
    query: &[f32],
    block: &[f32],
    dimension: usize,
) -> ([f32; PDX_BLOCK_SIZE], [f32; PDX_BLOCK_SIZE]) {
    let mut dot = [0.0_f32; PDX_BLOCK_SIZE];
    let mut norm_b_sq = [0.0_f32; PDX_BLOCK_SIZE];

    for d in 0..dimension {
        let q_d = query[d];
        let base = d * PDX_BLOCK_SIZE;
        for v in 0..PDX_BLOCK_SIZE {
            let b_val = block[base + v];
            dot[v] += q_d * b_val;
            norm_b_sq[v] += b_val * b_val;
        }
    }

    (dot, norm_b_sq)
}

/// Computes squared L2 norm of the query vector.
#[inline]
fn query_norm_squared(query: &[f32]) -> f32 {
    query.iter().map(|x| x * x).sum()
}

/// Converts fused dot+norms into cosine distances (1 - similarity).
#[inline]
fn finalize_cosine_distances(
    dot: [f32; PDX_BLOCK_SIZE],
    norm_b_sq: [f32; PDX_BLOCK_SIZE],
    query_norm_sq: f32,
    block_size: usize,
) -> [f32; PDX_BLOCK_SIZE] {
    let mut result = [0.0_f32; PDX_BLOCK_SIZE];
    for (v, result_v) in result.iter_mut().enumerate().take(block_size) {
        let denom_sq = query_norm_sq * norm_b_sq[v];
        if denom_sq < f32::EPSILON * f32::EPSILON {
            *result_v = 1.0; // Maximum distance for zero vectors
        } else {
            let sim = (dot[v] / denom_sq.sqrt()).clamp(-1.0, 1.0);
            *result_v = 1.0 - sim;
        }
    }
    result
}
