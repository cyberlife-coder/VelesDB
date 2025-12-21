//! Explicit SIMD optimizations using the `wide` crate for portable vectorization.
//!
//! This module provides SIMD-accelerated implementations of vector operations
//! that explicitly use SIMD instructions rather than relying on auto-vectorization.
//!
//! # Performance Goals
//!
//! - `dot_product_simd`: Target ≥10% faster than auto-vectorized version
//! - `cosine_similarity_simd`: Single-pass fused computation with SIMD
//! - `euclidean_distance_simd`: Vectorized squared difference accumulation
//!
//! # Architecture Support
//!
//! The `wide` crate automatically uses:
//! - **`x86_64`**: AVX2/SSE4.1/SSE2 (runtime detected)
//! - **ARM**: NEON
//! - **WASM**: SIMD128
//! - **Fallback**: Scalar operations

use wide::f32x8;

/// Computes dot product using explicit SIMD (8-wide f32 lanes).
///
/// # Algorithm
///
/// Processes 8 floats per iteration using SIMD multiply-accumulate,
/// then reduces horizontally.
///
/// # Panics
///
/// Panics if vectors have different lengths.
///
/// # Example
///
/// ```
/// use velesdb_core::simd_explicit::dot_product_simd;
///
/// let a = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
/// let b = vec![1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0];
/// let result = dot_product_simd(&a, &b);
/// assert!((result - 36.0).abs() < 1e-5);
/// ```
#[inline]
#[must_use]
pub fn dot_product_simd(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "Vector dimensions must match");

    let len = a.len();
    let simd_len = len / 8;
    let remainder = len % 8;

    let mut sum = f32x8::ZERO;

    // Process 8 elements at a time using FMA (fused multiply-add)
    // FMA provides better precision and can be faster on modern CPUs
    for i in 0..simd_len {
        let offset = i * 8;
        let va = f32x8::from(&a[offset..offset + 8]);
        let vb = f32x8::from(&b[offset..offset + 8]);
        sum = va.mul_add(vb, sum); // FMA: sum = (va * vb) + sum
    }

    // Horizontal sum of SIMD lanes
    let mut result = sum.reduce_add();

    // Handle remainder
    let base = simd_len * 8;
    for i in 0..remainder {
        result += a[base + i] * b[base + i];
    }

    result
}

/// Computes euclidean distance using explicit SIMD.
///
/// # Algorithm
///
/// Computes sqrt(sum((a[i] - b[i])²)) using SIMD for the squared differences.
///
/// # Panics
///
/// Panics if vectors have different lengths.
#[inline]
#[must_use]
pub fn euclidean_distance_simd(a: &[f32], b: &[f32]) -> f32 {
    squared_l2_distance_simd(a, b).sqrt()
}

/// Computes squared L2 distance using explicit SIMD.
///
/// Avoids the sqrt for comparison purposes (faster when only ranking matters).
///
/// # Panics
///
/// Panics if vectors have different lengths.
#[inline]
#[must_use]
pub fn squared_l2_distance_simd(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "Vector dimensions must match");

    let len = a.len();
    let simd_len = len / 8;
    let remainder = len % 8;

    let mut sum = f32x8::ZERO;

    for i in 0..simd_len {
        let offset = i * 8;
        let va = f32x8::from(&a[offset..offset + 8]);
        let vb = f32x8::from(&b[offset..offset + 8]);
        let diff = va - vb;
        sum = diff.mul_add(diff, sum); // FMA: sum = (diff * diff) + sum
    }

    let mut result = sum.reduce_add();

    let base = simd_len * 8;
    for i in 0..remainder {
        let diff = a[base + i] - b[base + i];
        result += diff * diff;
    }

    result
}

/// Computes cosine similarity using explicit SIMD with fused dot+norms.
///
/// # Algorithm
///
/// Single-pass computation of dot(a,b), norm(a)², norm(b)² using SIMD,
/// then: `dot / (sqrt(norm_a) * sqrt(norm_b))`
///
/// # Panics
///
/// Panics if vectors have different lengths.
#[inline]
#[must_use]
#[allow(clippy::similar_names)]
pub fn cosine_similarity_simd(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "Vector dimensions must match");

    let len = a.len();
    let simd_len = len / 8;
    let remainder = len % 8;

    let mut dot_sum = f32x8::ZERO;
    let mut norm_a_sum = f32x8::ZERO;
    let mut norm_b_sum = f32x8::ZERO;

    // FMA for all three accumulations - better precision and potentially faster
    for i in 0..simd_len {
        let offset = i * 8;
        let va = f32x8::from(&a[offset..offset + 8]);
        let vb = f32x8::from(&b[offset..offset + 8]);

        dot_sum = va.mul_add(vb, dot_sum);
        norm_a_sum = va.mul_add(va, norm_a_sum);
        norm_b_sum = vb.mul_add(vb, norm_b_sum);
    }

    let mut dot = dot_sum.reduce_add();
    let mut norm_a_sq = norm_a_sum.reduce_add();
    let mut norm_b_sq = norm_b_sum.reduce_add();

    // Handle remainder
    let base = simd_len * 8;
    for i in 0..remainder {
        let ai = a[base + i];
        let bi = b[base + i];
        dot += ai * bi;
        norm_a_sq += ai * ai;
        norm_b_sq += bi * bi;
    }

    let norm_a = norm_a_sq.sqrt();
    let norm_b = norm_b_sq.sqrt();

    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }

    dot / (norm_a * norm_b)
}

/// Computes the L2 norm (magnitude) of a vector using SIMD.
#[inline]
#[must_use]
pub fn norm_simd(v: &[f32]) -> f32 {
    let len = v.len();
    let simd_len = len / 8;
    let remainder = len % 8;

    let mut sum = f32x8::ZERO;

    for i in 0..simd_len {
        let offset = i * 8;
        let vv = f32x8::from(&v[offset..offset + 8]);
        sum = vv.mul_add(vv, sum); // FMA: sum = (vv * vv) + sum
    }

    let mut result = sum.reduce_add();

    let base = simd_len * 8;
    for i in 0..remainder {
        result += v[base + i] * v[base + i];
    }

    result.sqrt()
}

/// Computes Hamming distance for f32 binary vectors with loop unrolling.
///
/// Values > 0.5 are treated as 1, else 0. Counts differing positions.
/// Uses 8-wide loop unrolling for better cache utilization.
///
/// For packed binary data, use `hamming_distance_binary` which is ~50x faster.
///
/// # Panics
///
/// Panics if vectors have different lengths.
#[inline]
#[must_use]
pub fn hamming_distance_simd(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "Vector dimensions must match");

    let len = a.len();
    let chunks = len / 8;
    let remainder = len % 8;

    let mut count = 0u32;

    // Process 8 elements at a time for better cache/pipeline utilization
    for i in 0..chunks {
        let base = i * 8;
        count += u32::from((a[base] > 0.5) != (b[base] > 0.5));
        count += u32::from((a[base + 1] > 0.5) != (b[base + 1] > 0.5));
        count += u32::from((a[base + 2] > 0.5) != (b[base + 2] > 0.5));
        count += u32::from((a[base + 3] > 0.5) != (b[base + 3] > 0.5));
        count += u32::from((a[base + 4] > 0.5) != (b[base + 4] > 0.5));
        count += u32::from((a[base + 5] > 0.5) != (b[base + 5] > 0.5));
        count += u32::from((a[base + 6] > 0.5) != (b[base + 6] > 0.5));
        count += u32::from((a[base + 7] > 0.5) != (b[base + 7] > 0.5));
    }

    // Handle remainder
    let base = chunks * 8;
    for i in 0..remainder {
        if (a[base + i] > 0.5) != (b[base + i] > 0.5) {
            count += 1;
        }
    }

    #[allow(clippy::cast_precision_loss)]
    {
        count as f32
    }
}

/// Computes Hamming distance for packed binary vectors (u64 chunks).
///
/// Uses POPCNT for massive speedup on binary data. Each u64 contains 64 bits.
/// This is ~50x faster than f32-based Hamming for large binary vectors.
///
/// # Arguments
///
/// * `a` - First packed binary vector
/// * `b` - Second packed binary vector
///
/// # Returns
///
/// Number of differing bits.
///
/// # Panics
///
/// Panics if vectors have different lengths.
#[inline]
#[must_use]
pub fn hamming_distance_binary(a: &[u64], b: &[u64]) -> u32 {
    assert_eq!(a.len(), b.len(), "Vector dimensions must match");

    // Use iterator for better optimization - compiler can vectorize this
    a.iter()
        .zip(b.iter())
        .map(|(&x, &y)| (x ^ y).count_ones())
        .sum()
}

/// Computes Hamming distance for packed binary vectors with 8-wide unrolling.
///
/// Optimized version with explicit 8-wide loop unrolling for maximum throughput.
/// Use this for large vectors (>= 64 u64 elements).
///
/// # Panics
///
/// Panics if vectors have different lengths.
#[inline]
#[must_use]
pub fn hamming_distance_binary_fast(a: &[u64], b: &[u64]) -> u32 {
    assert_eq!(a.len(), b.len(), "Vector dimensions must match");

    let len = a.len();
    let chunks = len / 8;
    let remainder = len % 8;

    // Use multiple accumulators to exploit instruction-level parallelism
    let mut c0 = 0u32;
    let mut c1 = 0u32;
    let mut c2 = 0u32;
    let mut c3 = 0u32;

    for i in 0..chunks {
        let base = i * 8;
        c0 += (a[base] ^ b[base]).count_ones();
        c1 += (a[base + 1] ^ b[base + 1]).count_ones();
        c0 += (a[base + 2] ^ b[base + 2]).count_ones();
        c1 += (a[base + 3] ^ b[base + 3]).count_ones();
        c2 += (a[base + 4] ^ b[base + 4]).count_ones();
        c3 += (a[base + 5] ^ b[base + 5]).count_ones();
        c2 += (a[base + 6] ^ b[base + 6]).count_ones();
        c3 += (a[base + 7] ^ b[base + 7]).count_ones();
    }

    // Handle remainder
    let base = chunks * 8;
    for i in 0..remainder {
        c0 += (a[base + i] ^ b[base + i]).count_ones();
    }

    c0 + c1 + c2 + c3
}

/// Computes Jaccard similarity for f32 binary vectors with loop unrolling.
///
/// Values > 0.5 are treated as set members. Returns intersection/union.
///
/// # Panics
///
/// Panics if vectors have different lengths.
#[inline]
#[must_use]
pub fn jaccard_similarity_simd(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "Vector dimensions must match");

    let len = a.len();
    let chunks = len / 8;
    let remainder = len % 8;

    let mut intersection = 0u32;
    let mut union = 0u32;

    // Process 8 elements at a time
    for i in 0..chunks {
        let base = i * 8;
        for j in 0..8 {
            let ai = a[base + j] > 0.5;
            let bi = b[base + j] > 0.5;
            intersection += u32::from(ai && bi);
            union += u32::from(ai || bi);
        }
    }

    // Handle remainder
    let base = chunks * 8;
    for i in 0..remainder {
        let ai = a[base + i] > 0.5;
        let bi = b[base + i] > 0.5;
        intersection += u32::from(ai && bi);
        union += u32::from(ai || bi);
    }

    if union == 0 {
        return 1.0; // Empty sets are identical
    }

    #[allow(clippy::cast_precision_loss)]
    {
        intersection as f32 / union as f32
    }
}

/// Normalizes a vector in-place using SIMD.
#[inline]
pub fn normalize_inplace_simd(v: &mut [f32]) {
    let norm = norm_simd(v);

    if norm == 0.0 {
        return;
    }

    let inv_norm = 1.0 / norm;
    let inv_norm_simd = f32x8::splat(inv_norm);

    let len = v.len();
    let simd_len = len / 8;
    let remainder = len % 8;

    for i in 0..simd_len {
        let offset = i * 8;
        let vv = f32x8::from(&v[offset..offset + 8]);
        let normalized = vv * inv_norm_simd;
        let arr: [f32; 8] = normalized.into();
        v[offset..offset + 8].copy_from_slice(&arr);
    }

    let base = simd_len * 8;
    for i in 0..remainder {
        v[base + i] *= inv_norm;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPSILON: f32 = 1e-5;

    fn generate_test_vector(dim: usize, seed: f32) -> Vec<f32> {
        #[allow(clippy::cast_precision_loss)]
        (0..dim).map(|i| (seed + i as f32 * 0.1).sin()).collect()
    }

    // =========================================================================
    // Correctness Tests
    // =========================================================================

    #[test]
    fn test_dot_product_simd_basic() {
        let a = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
        let b = vec![1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0];
        let result = dot_product_simd(&a, &b);
        assert!((result - 36.0).abs() < EPSILON);
    }

    #[test]
    fn test_dot_product_simd_768d() {
        let a = generate_test_vector(768, 0.0);
        let b = generate_test_vector(768, 1.0);

        let simd_result = dot_product_simd(&a, &b);
        let scalar_result: f32 = a.iter().zip(&b).map(|(x, y)| x * y).sum();

        let rel_error = (simd_result - scalar_result).abs() / scalar_result.abs().max(1.0);
        assert!(rel_error < 1e-4, "Relative error too high: {rel_error}");
    }

    #[test]
    fn test_euclidean_distance_simd_identical() {
        let v = generate_test_vector(768, 0.0);
        let result = euclidean_distance_simd(&v, &v);
        assert!(
            result.abs() < EPSILON,
            "Identical vectors should have distance 0"
        );
    }

    #[test]
    fn test_euclidean_distance_simd_known() {
        let a = vec![0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        let b = vec![3.0, 4.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        let result = euclidean_distance_simd(&a, &b);
        assert!(
            (result - 5.0).abs() < EPSILON,
            "Expected 5.0 (3-4-5 triangle)"
        );
    }

    #[test]
    fn test_cosine_similarity_simd_identical() {
        let v = generate_test_vector(768, 0.0);
        let result = cosine_similarity_simd(&v, &v);
        assert!(
            (result - 1.0).abs() < EPSILON,
            "Identical vectors should have similarity 1.0"
        );
    }

    #[test]
    fn test_cosine_similarity_simd_orthogonal() {
        let mut a = vec![0.0; 16];
        let mut b = vec![0.0; 16];
        a[0] = 1.0;
        b[1] = 1.0;
        let result = cosine_similarity_simd(&a, &b);
        assert!(
            result.abs() < EPSILON,
            "Orthogonal vectors should have similarity 0"
        );
    }

    #[test]
    fn test_cosine_similarity_simd_opposite() {
        let a = generate_test_vector(768, 0.0);
        let b: Vec<f32> = a.iter().map(|x| -x).collect();
        let result = cosine_similarity_simd(&a, &b);
        assert!(
            (result + 1.0).abs() < EPSILON,
            "Opposite vectors should have similarity -1.0"
        );
    }

    #[test]
    fn test_normalize_inplace_simd_unit() {
        let mut v = vec![3.0, 4.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        normalize_inplace_simd(&mut v);

        let norm_after = norm_simd(&v);
        assert!((norm_after - 1.0).abs() < EPSILON, "Should be unit vector");
        assert!((v[0] - 0.6).abs() < EPSILON, "Expected 3/5 = 0.6");
        assert!((v[1] - 0.8).abs() < EPSILON, "Expected 4/5 = 0.8");
    }

    #[test]
    fn test_normalize_inplace_simd_zero() {
        let mut v = vec![0.0; 16];
        normalize_inplace_simd(&mut v);
        assert!(v.iter().all(|&x| x == 0.0), "Zero vector should stay zero");
    }

    // =========================================================================
    // Consistency with scalar implementation
    // =========================================================================

    #[test]
    fn test_consistency_with_scalar() {
        use crate::simd::{cosine_similarity_fast, dot_product_fast, euclidean_distance_fast};

        let a = generate_test_vector(768, 0.0);
        let b = generate_test_vector(768, 1.0);

        let dot_scalar = dot_product_fast(&a, &b);
        let dot_simd = dot_product_simd(&a, &b);
        assert!(
            (dot_scalar - dot_simd).abs() < 1e-3,
            "Dot product mismatch: {dot_scalar} vs {dot_simd}"
        );

        let dist_scalar = euclidean_distance_fast(&a, &b);
        let dist_simd = euclidean_distance_simd(&a, &b);
        assert!(
            (dist_scalar - dist_simd).abs() < 1e-3,
            "Euclidean distance mismatch: {dist_scalar} vs {dist_simd}"
        );

        let cos_scalar = cosine_similarity_fast(&a, &b);
        let cos_simd = cosine_similarity_simd(&a, &b);
        assert!(
            (cos_scalar - cos_simd).abs() < 1e-5,
            "Cosine similarity mismatch: {cos_scalar} vs {cos_simd}"
        );
    }

    // =========================================================================
    // Edge cases
    // =========================================================================

    #[test]
    fn test_odd_dimensions() {
        let a = vec![1.0, 2.0, 3.0, 4.0, 5.0]; // 5 elements (not multiple of 8)
        let b = vec![5.0, 4.0, 3.0, 2.0, 1.0];

        let result = dot_product_simd(&a, &b);
        let expected: f32 = a.iter().zip(&b).map(|(x, y)| x * y).sum();
        assert!((result - expected).abs() < EPSILON);
    }

    #[test]
    fn test_small_vectors() {
        let a = vec![3.0];
        let b = vec![4.0];
        assert!((dot_product_simd(&a, &b) - 12.0).abs() < EPSILON);
    }

    #[test]
    #[should_panic(expected = "Vector dimensions must match")]
    fn test_dimension_mismatch() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![1.0, 2.0];
        let _ = dot_product_simd(&a, &b);
    }

    // =========================================================================
    // Hamming distance tests
    // =========================================================================

    #[test]
    fn test_hamming_distance_simd_identical() {
        let a = vec![1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0];
        let result = hamming_distance_simd(&a, &a);
        assert!(
            result.abs() < EPSILON,
            "Identical vectors should have distance 0"
        );
    }

    #[test]
    fn test_hamming_distance_simd_all_different() {
        let a = vec![1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0];
        let b = vec![0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0];
        let result = hamming_distance_simd(&a, &b);
        assert!((result - 8.0).abs() < EPSILON, "All different = 8");
    }

    #[test]
    fn test_hamming_distance_simd_partial() {
        let a = vec![1.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0];
        // Differences at positions 1, 3, 5, 7 = 4 differences
        let result = hamming_distance_simd(&a, &b);
        assert!((result - 4.0).abs() < EPSILON, "Expected 4 differences");
    }

    #[test]
    fn test_hamming_distance_simd_consistency() {
        use crate::simd::hamming_distance_fast;

        let a: Vec<f32> = (0..768)
            .map(|i| if i % 3 == 0 { 1.0 } else { 0.0 })
            .collect();
        let b: Vec<f32> = (0..768)
            .map(|i| if i % 2 == 0 { 1.0 } else { 0.0 })
            .collect();

        let scalar = hamming_distance_fast(&a, &b);
        let simd = hamming_distance_simd(&a, &b);

        assert!(
            (scalar - simd).abs() < 1.0,
            "Hamming mismatch: {scalar} vs {simd}"
        );
    }

    // =========================================================================
    // Binary Hamming distance tests (u64 packed)
    // =========================================================================

    #[test]
    fn test_hamming_distance_binary_identical() {
        let a = vec![0xFFFF_FFFF_FFFF_FFFFu64; 16];
        let result = hamming_distance_binary(&a, &a);
        assert_eq!(result, 0, "Identical should be 0");
    }

    #[test]
    fn test_hamming_distance_binary_all_different() {
        let a = vec![0u64; 1];
        let b = vec![0xFFFF_FFFF_FFFF_FFFFu64; 1];
        let result = hamming_distance_binary(&a, &b);
        assert_eq!(result, 64, "All 64 bits different");
    }

    #[test]
    fn test_hamming_distance_binary_known() {
        let a = vec![0b1010_1010u64];
        let b = vec![0b0101_0101u64];
        let result = hamming_distance_binary(&a, &b);
        assert_eq!(result, 8, "8 bits different in low byte");
    }

    #[test]
    fn test_hamming_distance_binary_fast_identical() {
        let a = vec![0xFFFF_FFFF_FFFF_FFFFu64; 16];
        let result = hamming_distance_binary_fast(&a, &a);
        assert_eq!(result, 0, "Identical should be 0");
    }

    #[test]
    fn test_hamming_distance_binary_fast_all_different() {
        let a = vec![0u64; 16];
        let b = vec![0xFFFF_FFFF_FFFF_FFFFu64; 16];
        let result = hamming_distance_binary_fast(&a, &b);
        assert_eq!(result, 64 * 16, "All bits different");
    }

    #[test]
    fn test_hamming_distance_binary_fast_consistency() {
        let a: Vec<u64> = (0..24).map(|i| i * 0x1234_5678).collect();
        let b: Vec<u64> = (0..24).map(|i| i * 0x8765_4321).collect();

        let standard = hamming_distance_binary(&a, &b);
        let fast = hamming_distance_binary_fast(&a, &b);

        assert_eq!(standard, fast, "Fast should match standard");
    }

    // =========================================================================
    // Jaccard similarity tests
    // =========================================================================

    #[test]
    fn test_jaccard_similarity_simd_identical() {
        let a = vec![1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0];
        let result = jaccard_similarity_simd(&a, &a);
        assert!((result - 1.0).abs() < EPSILON, "Identical = 1.0");
    }

    #[test]
    fn test_jaccard_similarity_simd_disjoint() {
        let a = vec![1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        let b = vec![0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0];
        let result = jaccard_similarity_simd(&a, &b);
        assert!(result.abs() < EPSILON, "Disjoint sets = 0.0");
    }

    #[test]
    fn test_jaccard_similarity_simd_half_overlap() {
        let a = vec![1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        // Intersection = 1 (position 0), Union = 3 (positions 0, 1, 2)
        let result = jaccard_similarity_simd(&a, &b);
        assert!((result - (1.0 / 3.0)).abs() < EPSILON, "Expected 1/3");
    }

    #[test]
    fn test_jaccard_similarity_simd_empty() {
        let a = vec![0.0; 16];
        let b = vec![0.0; 16];
        let result = jaccard_similarity_simd(&a, &b);
        assert!((result - 1.0).abs() < EPSILON, "Empty sets = 1.0");
    }

    #[test]
    fn test_jaccard_similarity_simd_consistency() {
        use crate::simd::jaccard_similarity_fast;

        let a: Vec<f32> = (0..768)
            .map(|i| if i % 3 == 0 { 1.0 } else { 0.0 })
            .collect();
        let b: Vec<f32> = (0..768)
            .map(|i| if i % 2 == 0 { 1.0 } else { 0.0 })
            .collect();

        let scalar = jaccard_similarity_fast(&a, &b);
        let simd = jaccard_similarity_simd(&a, &b);

        assert!(
            (scalar - simd).abs() < 1e-4,
            "Jaccard mismatch: {scalar} vs {simd}"
        );
    }
}
