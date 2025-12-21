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

    // Process 8 elements at a time
    for i in 0..simd_len {
        let offset = i * 8;
        let va = f32x8::from(&a[offset..offset + 8]);
        let vb = f32x8::from(&b[offset..offset + 8]);
        sum += va * vb;
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
        sum += diff * diff;
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

    for i in 0..simd_len {
        let offset = i * 8;
        let va = f32x8::from(&a[offset..offset + 8]);
        let vb = f32x8::from(&b[offset..offset + 8]);

        dot_sum += va * vb;
        norm_a_sum += va * va;
        norm_b_sum += vb * vb;
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
        sum += vv * vv;
    }

    let mut result = sum.reduce_add();

    let base = simd_len * 8;
    for i in 0..remainder {
        result += v[base + i] * v[base + i];
    }

    result.sqrt()
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
}
