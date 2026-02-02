//! SIMD-optimized distance calculations for WASM.
//!
//! Uses the `wide` crate which automatically uses WASM SIMD128 when available.

#![allow(dead_code)] // Reason: Functions reserved for future WASM SIMD optimization paths

use wide::f32x8;

/// Computes dot product using SIMD.
#[inline]
pub fn dot_product(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len());

    let len = a.len();
    let simd_len = len / 8;

    let mut sum = f32x8::ZERO;

    for i in 0..simd_len {
        let offset = i * 8;
        let va = f32x8::from(&a[offset..offset + 8]);
        let vb = f32x8::from(&b[offset..offset + 8]);
        sum = va.mul_add(vb, sum);
    }

    let mut result = sum.reduce_add();

    // Handle remainder
    let base = simd_len * 8;
    for i in base..len {
        result += a[i] * b[i];
    }

    result
}

/// Computes Euclidean distance using SIMD.
#[inline]
pub fn euclidean_distance(a: &[f32], b: &[f32]) -> f32 {
    squared_l2(a, b).sqrt()
}

/// Computes squared L2 distance using SIMD.
#[inline]
pub fn squared_l2(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len());

    let len = a.len();
    let simd_len = len / 8;

    let mut sum = f32x8::ZERO;

    for i in 0..simd_len {
        let offset = i * 8;
        let va = f32x8::from(&a[offset..offset + 8]);
        let vb = f32x8::from(&b[offset..offset + 8]);
        let diff = va - vb;
        sum = diff.mul_add(diff, sum);
    }

    let mut result = sum.reduce_add();

    let base = simd_len * 8;
    for i in base..len {
        let diff = a[i] - b[i];
        result += diff * diff;
    }

    result
}

/// Computes cosine similarity using SIMD.
#[inline]
#[allow(clippy::similar_names)]
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len());

    let len = a.len();
    let simd_len = len / 8;

    let mut dot_sum = f32x8::ZERO;
    let mut norm_a_sum = f32x8::ZERO;
    let mut norm_b_sum = f32x8::ZERO;

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
    for i in base..len {
        let ai = a[i];
        let bi = b[i];
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

/// Computes Hamming distance (count of differing elements).
/// Treats non-zero values as 1, zero as 0.
#[inline]
pub fn hamming_distance(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len());

    let mut count = 0u32;
    for i in 0..a.len() {
        let bit_a = u32::from(a[i] != 0.0);
        let bit_b = u32::from(b[i] != 0.0);
        if bit_a != bit_b {
            count += 1;
        }
    }
    #[allow(clippy::cast_precision_loss)]
    {
        count as f32
    }
}

/// Computes Jaccard similarity (intersection / union).
/// Treats non-zero values as set membership.
#[inline]
pub fn jaccard_similarity(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len());

    let mut intersection = 0u32;
    let mut union = 0u32;

    for i in 0..a.len() {
        let in_a = a[i] != 0.0;
        let in_b = b[i] != 0.0;

        if in_a && in_b {
            intersection += 1;
        }
        if in_a || in_b {
            union += 1;
        }
    }

    if union == 0 {
        return 1.0; // Both empty sets are identical
    }

    #[allow(clippy::cast_precision_loss)]
    {
        intersection as f32 / union as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPSILON: f32 = 1e-5;

    #[test]
    fn test_dot_product_basic() {
        let a = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
        let b = vec![1.0; 8];
        let result = dot_product(&a, &b);
        assert!((result - 36.0).abs() < EPSILON);
    }

    #[test]
    fn test_euclidean_345() {
        let a = vec![0.0, 0.0, 0.0];
        let b = vec![3.0, 4.0, 0.0];
        let result = euclidean_distance(&a, &b);
        assert!((result - 5.0).abs() < EPSILON);
    }

    #[test]
    #[allow(clippy::cast_precision_loss)]
    fn test_cosine_identical() {
        let a: Vec<f32> = (0..768).map(|i| (i as f32 * 0.1).sin()).collect();
        let result = cosine_similarity(&a, &a);
        assert!((result - 1.0).abs() < EPSILON);
    }

    #[test]
    fn test_cosine_orthogonal() {
        let mut a = vec![0.0; 16];
        let mut b = vec![0.0; 16];
        a[0] = 1.0;
        b[1] = 1.0;
        let result = cosine_similarity(&a, &b);
        assert!(result.abs() < EPSILON);
    }

    #[test]
    fn test_odd_dimensions() {
        let a = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let b = vec![5.0, 4.0, 3.0, 2.0, 1.0];
        let result = dot_product(&a, &b);
        let expected: f32 = a.iter().zip(&b).map(|(x, y)| x * y).sum();
        assert!((result - expected).abs() < EPSILON);
    }

    #[test]
    fn test_hamming_identical() {
        let a = vec![1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0];
        let result = hamming_distance(&a, &a);
        assert!((result - 0.0).abs() < EPSILON);
    }

    #[test]
    fn test_hamming_all_different() {
        let a = vec![1.0, 0.0, 1.0, 0.0];
        let b = vec![0.0, 1.0, 0.0, 1.0];
        let result = hamming_distance(&a, &b);
        assert!((result - 4.0).abs() < EPSILON);
    }

    #[test]
    fn test_hamming_partial() {
        let a = vec![1.0, 1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0, 1.0];
        let result = hamming_distance(&a, &b);
        assert!((result - 2.0).abs() < EPSILON);
    }

    #[test]
    fn test_jaccard_identical() {
        let a = vec![1.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0];
        let result = jaccard_similarity(&a, &a);
        assert!((result - 1.0).abs() < EPSILON);
    }

    #[test]
    fn test_jaccard_disjoint() {
        let a = vec![1.0, 1.0, 0.0, 0.0];
        let b = vec![0.0, 0.0, 1.0, 1.0];
        let result = jaccard_similarity(&a, &b);
        assert!((result - 0.0).abs() < EPSILON);
    }

    #[test]
    fn test_jaccard_half_overlap() {
        let a = vec![1.0, 1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 1.0, 0.0];
        let result = jaccard_similarity(&a, &b);
        // intersection=1, union=3 -> 1/3 ≈ 0.333
        assert!((result - 1.0 / 3.0).abs() < EPSILON);
    }

    #[test]
    fn test_jaccard_empty_sets() {
        let a = vec![0.0, 0.0, 0.0, 0.0];
        let b = vec![0.0, 0.0, 0.0, 0.0];
        let result = jaccard_similarity(&a, &b);
        assert!((result - 1.0).abs() < EPSILON); // Both empty = identical
    }
}
