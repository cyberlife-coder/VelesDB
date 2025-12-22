//! SIMD-optimized vector operations for high-performance distance calculations.
//!
//! # Performance (WIS-45 validated)
//!
//! - `cosine_similarity_fast`: ~83ns for 768d (using explicit SIMD)
//! - `euclidean_distance_fast`: ~47ns for 768d (using explicit SIMD)
//! - `dot_product_fast`: ~45ns for 768d (using explicit SIMD)
//!
//! # Implementation Strategy
//!
//! This module delegates to `simd_explicit` for all distance functions,
//! using the `wide` crate for portable SIMD (AVX2/NEON/WASM).
//!
//! # Note on `hnsw_rs` Integration
//!
//! Custom `Distance` trait implementations for `hnsw_rs` are NOT supported due to
//! undocumented internal invariants in the library. The SIMD functions in this module
//! are used by `DistanceMetric::calculate()` for direct distance computations outside
//! of the HNSW index.

use crate::simd_explicit;

/// Computes cosine similarity using explicit SIMD (f32x8).
///
/// # Algorithm
///
/// Single-pass fused computation of dot(a,b), norm(a)², norm(b)² using SIMD FMA.
/// Result: `dot / (sqrt(norm_a) * sqrt(norm_b))`
///
/// # Performance
///
/// ~83ns for 768d vectors (3.9x faster than auto-vectorized version).
///
/// # Panics
///
/// Panics if vectors have different lengths.
#[inline]
#[must_use]
pub fn cosine_similarity_fast(a: &[f32], b: &[f32]) -> f32 {
    simd_explicit::cosine_similarity_simd(a, b)
}

/// Computes euclidean distance using explicit SIMD (f32x8).
///
/// # Performance
///
/// ~47ns for 768d vectors (2.9x faster than auto-vectorized version).
///
/// # Panics
///
/// Panics if vectors have different lengths.
#[inline]
#[must_use]
pub fn euclidean_distance_fast(a: &[f32], b: &[f32]) -> f32 {
    simd_explicit::euclidean_distance_simd(a, b)
}

/// Computes squared L2 distance (avoids sqrt for comparison purposes).
///
/// # Panics
///
/// Panics if vectors have different lengths.
#[inline]
#[must_use]
pub fn squared_l2_distance(a: &[f32], b: &[f32]) -> f32 {
    simd_explicit::squared_l2_distance_simd(a, b)
}

/// Normalizes a vector in-place using explicit SIMD.
///
/// # Panics
///
/// Does not panic on zero vector (leaves unchanged).
#[inline]
pub fn normalize_inplace(v: &mut [f32]) {
    simd_explicit::normalize_inplace_simd(v);
}

/// Computes the L2 norm (magnitude) of a vector.
#[inline]
#[must_use]
pub fn norm(v: &[f32]) -> f32 {
    v.iter().map(|x| x * x).sum::<f32>().sqrt()
}

/// Computes dot product using explicit SIMD (f32x8).
///
/// # Performance
///
/// ~45ns for 768d vectors (2.9x faster than auto-vectorized version).
///
/// # Panics
///
/// Panics if vectors have different lengths.
#[inline]
#[must_use]
pub fn dot_product_fast(a: &[f32], b: &[f32]) -> f32 {
    simd_explicit::dot_product_simd(a, b)
}

/// Computes Hamming distance for binary vectors.
///
/// Counts the number of positions where values differ (treating values > 0.5 as 1, else 0).
///
/// # Arguments
///
/// * `a` - First binary vector (values > 0.5 treated as 1)
/// * `b` - Second binary vector (values > 0.5 treated as 1)
///
/// # Returns
///
/// Number of positions where bits differ.
///
/// # Panics
///
/// Panics if vectors have different lengths.
#[inline]
#[must_use]
pub fn hamming_distance_fast(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "Vector dimensions must match");

    let chunks = a.len() / 4;
    let remainder = a.len() % 4;

    let mut count0 = 0u32;
    let mut count1 = 0u32;
    let mut count2 = 0u32;
    let mut count3 = 0u32;

    for i in 0..chunks {
        let base = i * 4;
        // Convert to binary: > 0.5 = 1, else 0
        let a0 = a[base] > 0.5;
        let a1 = a[base + 1] > 0.5;
        let a2 = a[base + 2] > 0.5;
        let a3 = a[base + 3] > 0.5;

        let b0 = b[base] > 0.5;
        let b1 = b[base + 1] > 0.5;
        let b2 = b[base + 2] > 0.5;
        let b3 = b[base + 3] > 0.5;

        // XOR to find differences
        count0 += u32::from(a0 != b0);
        count1 += u32::from(a1 != b1);
        count2 += u32::from(a2 != b2);
        count3 += u32::from(a3 != b3);
    }

    // Handle remainder
    let base = chunks * 4;
    for i in 0..remainder {
        let ai = a[base + i] > 0.5;
        let bi = b[base + i] > 0.5;
        count0 += u32::from(ai != bi);
    }

    #[allow(clippy::cast_precision_loss)]
    // Intentional: hamming distance won't exceed 2^23 in practice
    {
        (count0 + count1 + count2 + count3) as f32
    }
}

/// Computes Jaccard similarity for set-like vectors.
///
/// Measures intersection over union of non-zero elements.
/// Values > 0.5 are considered "in the set".
///
/// # Arguments
///
/// * `a` - First set vector (values > 0.5 treated as set members)
/// * `b` - Second set vector (values > 0.5 treated as set members)
///
/// # Returns
///
/// Jaccard similarity in range [0.0, 1.0]. Returns 1.0 for two empty sets.
///
/// # Panics
///
/// Panics if vectors have different lengths.
#[inline]
#[must_use]
pub fn jaccard_similarity_fast(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "Vector dimensions must match");

    let mut intersection = 0u32;
    let mut union = 0u32;

    for i in 0..a.len() {
        let in_a = a[i] > 0.5;
        let in_b = b[i] > 0.5;

        if in_a && in_b {
            intersection += 1;
        }
        if in_a || in_b {
            union += 1;
        }
    }

    // Empty sets are defined as identical (similarity = 1.0)
    if union == 0 {
        return 1.0;
    }

    #[allow(clippy::cast_precision_loss)] // Intentional: set size won't exceed 2^23 in practice
    {
        intersection as f32 / union as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // TDD Tests - Written BEFORE optimization (RED phase)
    // These define the expected behavior and performance contracts.
    // =========================================================================

    const EPSILON: f32 = 1e-5;

    fn generate_test_vector(dim: usize, seed: f32) -> Vec<f32> {
        #[allow(clippy::cast_precision_loss)]
        (0..dim).map(|i| (seed + i as f32 * 0.1).sin()).collect()
    }

    // --- Correctness Tests ---

    #[test]
    fn test_cosine_similarity_identical_vectors() {
        let v = vec![1.0, 2.0, 3.0, 4.0];
        let result = cosine_similarity_fast(&v, &v);
        assert!(
            (result - 1.0).abs() < EPSILON,
            "Identical vectors should have similarity 1.0"
        );
    }

    #[test]
    fn test_cosine_similarity_orthogonal_vectors() {
        let a = vec![1.0, 0.0, 0.0, 0.0];
        let b = vec![0.0, 1.0, 0.0, 0.0];
        let result = cosine_similarity_fast(&a, &b);
        assert!(
            result.abs() < EPSILON,
            "Orthogonal vectors should have similarity 0.0"
        );
    }

    #[test]
    fn test_cosine_similarity_opposite_vectors() {
        let a = vec![1.0, 2.0, 3.0, 4.0];
        let b: Vec<f32> = a.iter().map(|x| -x).collect();
        let result = cosine_similarity_fast(&a, &b);
        assert!(
            (result + 1.0).abs() < EPSILON,
            "Opposite vectors should have similarity -1.0"
        );
    }

    #[test]
    fn test_cosine_similarity_zero_vector() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![0.0, 0.0, 0.0];
        let result = cosine_similarity_fast(&a, &b);
        assert!(result.abs() < EPSILON, "Zero vector should return 0.0");
    }

    #[test]
    fn test_euclidean_distance_identical_vectors() {
        let v = vec![1.0, 2.0, 3.0, 4.0];
        let result = euclidean_distance_fast(&v, &v);
        assert!(
            result.abs() < EPSILON,
            "Identical vectors should have distance 0.0"
        );
    }

    #[test]
    fn test_euclidean_distance_known_value() {
        let a = vec![0.0, 0.0, 0.0];
        let b = vec![3.0, 4.0, 0.0];
        let result = euclidean_distance_fast(&a, &b);
        assert!(
            (result - 5.0).abs() < EPSILON,
            "Expected distance 5.0 (3-4-5 triangle)"
        );
    }

    #[test]
    fn test_euclidean_distance_768d() {
        let a = generate_test_vector(768, 0.0);
        let b = generate_test_vector(768, 1.0);

        let result = euclidean_distance_fast(&a, &b);

        // Compare with naive implementation
        let expected: f32 = a
            .iter()
            .zip(&b)
            .map(|(x, y)| (x - y).powi(2))
            .sum::<f32>()
            .sqrt();

        assert!(
            (result - expected).abs() < EPSILON,
            "Should match naive implementation"
        );
    }

    #[test]
    fn test_dot_product_fast_correctness() {
        let a = vec![1.0, 2.0, 3.0, 4.0];
        let b = vec![5.0, 6.0, 7.0, 8.0];
        let result = dot_product_fast(&a, &b);
        let expected = 1.0 * 5.0 + 2.0 * 6.0 + 3.0 * 7.0 + 4.0 * 8.0; // 70.0
        assert!((result - expected).abs() < EPSILON);
    }

    #[test]
    fn test_dot_product_fast_768d() {
        let a = generate_test_vector(768, 0.0);
        let b = generate_test_vector(768, 1.0);

        let result = dot_product_fast(&a, &b);
        let expected: f32 = a.iter().zip(&b).map(|(x, y)| x * y).sum();

        // Relax epsilon for high-dimensional accumulated floating point errors
        let rel_error = (result - expected).abs() / expected.abs().max(1.0);
        assert!(rel_error < 1e-4, "Relative error too high: {rel_error}");
    }

    #[test]
    fn test_normalize_inplace_unit_vector() {
        let mut v = vec![3.0, 4.0, 0.0];
        normalize_inplace(&mut v);

        let norm_after: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!(
            (norm_after - 1.0).abs() < EPSILON,
            "Normalized vector should have unit norm"
        );
        assert!((v[0] - 0.6).abs() < EPSILON, "Expected 3/5 = 0.6");
        assert!((v[1] - 0.8).abs() < EPSILON, "Expected 4/5 = 0.8");
    }

    #[test]
    fn test_normalize_inplace_zero_vector() {
        let mut v = vec![0.0, 0.0, 0.0];
        normalize_inplace(&mut v);
        // Should not panic, vector unchanged
        assert!(v.iter().all(|&x| x == 0.0));
    }

    #[test]
    fn test_normalize_inplace_768d() {
        let mut v = generate_test_vector(768, 0.0);
        normalize_inplace(&mut v);

        let norm_after: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!(
            (norm_after - 1.0).abs() < EPSILON,
            "Should be unit vector after normalization"
        );
    }

    // --- Consistency Tests (fast vs baseline) ---

    #[test]
    fn test_cosine_consistency_with_baseline() {
        let a = generate_test_vector(768, 0.0);
        let b = generate_test_vector(768, 1.0);

        // Baseline (3-pass)
        let dot: f32 = a.iter().zip(&b).map(|(x, y)| x * y).sum();
        let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
        let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
        let baseline = dot / (norm_a * norm_b);

        // Fast (single-pass fused)
        let fast = cosine_similarity_fast(&a, &b);

        assert!(
            (fast - baseline).abs() < EPSILON,
            "Fast implementation should match baseline: {fast} vs {baseline}"
        );
    }

    // --- Edge Cases ---

    #[test]
    fn test_odd_dimension_vectors() {
        // Test non-multiple-of-4 dimensions
        let a = vec![1.0, 2.0, 3.0, 4.0, 5.0]; // 5 elements
        let b = vec![5.0, 4.0, 3.0, 2.0, 1.0];

        let dot = dot_product_fast(&a, &b);
        let expected = 1.0 * 5.0 + 2.0 * 4.0 + 3.0 * 3.0 + 4.0 * 2.0 + 5.0 * 1.0; // 35.0
        assert!((dot - expected).abs() < EPSILON);

        let dist = euclidean_distance_fast(&a, &b);
        let expected_dist: f32 = a
            .iter()
            .zip(&b)
            .map(|(x, y)| (x - y).powi(2))
            .sum::<f32>()
            .sqrt();
        assert!((dist - expected_dist).abs() < EPSILON);
    }

    #[test]
    fn test_small_vectors() {
        // Single element
        let a = vec![3.0];
        let b = vec![4.0];
        assert!((dot_product_fast(&a, &b) - 12.0).abs() < EPSILON);
        assert!((euclidean_distance_fast(&a, &b) - 1.0).abs() < EPSILON);

        // Two elements
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        assert!((cosine_similarity_fast(&a, &b)).abs() < EPSILON);
    }

    #[test]
    #[should_panic(expected = "Vector dimensions must match")]
    fn test_dimension_mismatch_panics() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![1.0, 2.0];
        let _ = cosine_similarity_fast(&a, &b);
    }

    // --- norm() tests ---

    #[test]
    fn test_norm_zero_vector() {
        let v = vec![0.0, 0.0, 0.0];
        assert!(norm(&v).abs() < EPSILON);
    }

    #[test]
    fn test_norm_unit_vector() {
        let v = vec![1.0, 0.0, 0.0];
        assert!((norm(&v) - 1.0).abs() < EPSILON);
    }

    #[test]
    fn test_norm_known_value() {
        let v = vec![3.0, 4.0];
        assert!((norm(&v) - 5.0).abs() < EPSILON);
    }

    // --- squared_l2_distance tests ---

    #[test]
    fn test_squared_l2_identical() {
        let v = vec![1.0, 2.0, 3.0];
        assert!(squared_l2_distance(&v, &v).abs() < EPSILON);
    }

    #[test]
    fn test_squared_l2_known_value() {
        let a = vec![0.0, 0.0];
        let b = vec![3.0, 4.0];
        assert!((squared_l2_distance(&a, &b) - 25.0).abs() < EPSILON);
    }

    // --- hamming_distance_fast tests ---

    #[test]
    fn test_hamming_identical() {
        let a = vec![1.0, 0.0, 1.0, 0.0];
        assert!(hamming_distance_fast(&a, &a).abs() < EPSILON);
    }

    #[test]
    fn test_hamming_all_different() {
        let a = vec![1.0, 0.0, 1.0, 0.0];
        let b = vec![0.0, 1.0, 0.0, 1.0];
        assert!((hamming_distance_fast(&a, &b) - 4.0).abs() < EPSILON);
    }

    #[test]
    fn test_hamming_partial() {
        let a = vec![1.0, 1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0, 1.0];
        assert!((hamming_distance_fast(&a, &b) - 2.0).abs() < EPSILON);
    }

    #[test]
    fn test_hamming_odd_dimension() {
        let a = vec![1.0, 0.0, 1.0, 0.0, 1.0];
        let b = vec![0.0, 0.0, 1.0, 1.0, 1.0];
        assert!((hamming_distance_fast(&a, &b) - 2.0).abs() < EPSILON);
    }

    // --- jaccard_similarity_fast tests ---

    #[test]
    fn test_jaccard_identical() {
        let a = vec![1.0, 0.0, 1.0, 0.0];
        assert!((jaccard_similarity_fast(&a, &a) - 1.0).abs() < EPSILON);
    }

    #[test]
    fn test_jaccard_disjoint() {
        let a = vec![1.0, 0.0, 0.0, 0.0];
        let b = vec![0.0, 1.0, 0.0, 0.0];
        assert!(jaccard_similarity_fast(&a, &b).abs() < EPSILON);
    }

    #[test]
    fn test_jaccard_half_overlap() {
        let a = vec![1.0, 1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 1.0, 0.0];
        // Intersection: 1, Union: 3
        assert!((jaccard_similarity_fast(&a, &b) - (1.0 / 3.0)).abs() < EPSILON);
    }

    #[test]
    fn test_jaccard_empty_sets() {
        let a = vec![0.0, 0.0, 0.0, 0.0];
        let b = vec![0.0, 0.0, 0.0, 0.0];
        assert!((jaccard_similarity_fast(&a, &b) - 1.0).abs() < EPSILON);
    }
}
