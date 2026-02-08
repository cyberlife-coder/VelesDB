#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::float_cmp,
    clippy::approx_constant
)]
//! Tests for `simd` module - SIMD-optimized distance calculations.

use crate::simd_native::{
    calculate_prefetch_distance, cosine_similarity_native, dot_product_native, euclidean_native,
    hamming_distance_native, jaccard_similarity_native, norm_native, normalize_inplace_native,
    prefetch_vector, squared_l2_native, L2_CACHE_LINE_BYTES,
};

// =========================================================================
// TDD Tests - Written BEFORE optimization (RED phase)
// These define the expected behavior and performance contracts.
// =========================================================================

const EPSILON: f32 = 1e-5;

#[allow(clippy::cast_precision_loss)]
fn generate_test_vector(dim: usize, seed: f32) -> Vec<f32> {
    (0..dim).map(|i| (seed + i as f32 * 0.1).sin()).collect()
}

// --- Correctness Tests ---

#[test]
fn test_cosine_similarity_identical_vectors() {
    let v = vec![1.0, 2.0, 3.0, 4.0];
    let result = cosine_similarity_native(&v, &v);
    assert!(
        (result - 1.0).abs() < EPSILON,
        "Identical vectors should have similarity 1.0"
    );
}

#[test]
fn test_cosine_similarity_orthogonal_vectors() {
    let a = vec![1.0, 0.0, 0.0, 0.0];
    let b = vec![0.0, 1.0, 0.0, 0.0];
    let result = cosine_similarity_native(&a, &b);
    assert!(
        result.abs() < EPSILON,
        "Orthogonal vectors should have similarity 0.0"
    );
}

#[test]
fn test_cosine_similarity_opposite_vectors() {
    let a = vec![1.0, 2.0, 3.0, 4.0];
    let b: Vec<f32> = a.iter().map(|x| -x).collect();
    let result = cosine_similarity_native(&a, &b);
    assert!(
        (result + 1.0).abs() < EPSILON,
        "Opposite vectors should have similarity -1.0"
    );
}

#[test]
fn test_cosine_similarity_zero_vector() {
    let a = vec![1.0, 2.0, 3.0];
    let b = vec![0.0, 0.0, 0.0];
    let result = cosine_similarity_native(&a, &b);
    assert!(result.abs() < EPSILON, "Zero vector should return 0.0");
}

#[test]
fn test_euclidean_distance_identical_vectors() {
    let v = vec![1.0, 2.0, 3.0, 4.0];
    let result = euclidean_native(&v, &v);
    assert!(
        result.abs() < EPSILON,
        "Identical vectors should have distance 0.0"
    );
}

#[test]
fn test_euclidean_distance_known_value() {
    let a = vec![0.0, 0.0, 0.0];
    let b = vec![3.0, 4.0, 0.0];
    let result = euclidean_native(&a, &b);
    assert!(
        (result - 5.0).abs() < EPSILON,
        "Expected distance 5.0 (3-4-5 triangle)"
    );
}

#[test]
fn test_euclidean_distance_768d() {
    let a = generate_test_vector(768, 0.0);
    let b = generate_test_vector(768, 1.0);

    let result = euclidean_native(&a, &b);

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
    let result = dot_product_native(&a, &b);
    let expected = 1.0 * 5.0 + 2.0 * 6.0 + 3.0 * 7.0 + 4.0 * 8.0; // 70.0
    assert!((result - expected).abs() < EPSILON);
}

#[test]
fn test_dot_product_fast_768d() {
    let a = generate_test_vector(768, 0.0);
    let b = generate_test_vector(768, 1.0);

    let result = dot_product_native(&a, &b);
    let expected: f32 = a.iter().zip(&b).map(|(x, y)| x * y).sum();

    // Relax epsilon for high-dimensional accumulated floating point errors
    let rel_error = (result - expected).abs() / expected.abs().max(1.0);
    assert!(rel_error < 1e-4, "Relative error too high: {rel_error}");
}

#[test]
fn test_normalize_inplace_unit_vector() {
    let mut v = vec![3.0, 4.0, 0.0];
    normalize_inplace_native(&mut v);

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
    normalize_inplace_native(&mut v);
    // Should not panic, vector unchanged
    assert!(v.iter().all(|&x| x == 0.0));
}

#[test]
fn test_normalize_inplace_768d() {
    let mut v = generate_test_vector(768, 0.0);
    normalize_inplace_native(&mut v);

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
    let fast = cosine_similarity_native(&a, &b);

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

    let dot = dot_product_native(&a, &b);
    let expected = 1.0 * 5.0 + 2.0 * 4.0 + 3.0 * 3.0 + 4.0 * 2.0 + 5.0 * 1.0; // 35.0
    assert!((dot - expected).abs() < EPSILON);

    let dist = euclidean_native(&a, &b);
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
    assert!((dot_product_native(&a, &b) - 12.0).abs() < EPSILON);
    assert!((euclidean_native(&a, &b) - 1.0).abs() < EPSILON);

    // Two elements
    let a = vec![1.0, 0.0];
    let b = vec![0.0, 1.0];
    assert!((cosine_similarity_native(&a, &b)).abs() < EPSILON);
}

#[test]
#[should_panic(expected = "Vector dimensions must match")]
fn test_dimension_mismatch_panics() {
    let a = vec![1.0, 2.0, 3.0];
    let b = vec![1.0, 2.0];
    let _ = cosine_similarity_native(&a, &b);
}

// --- norm_native() tests ---

#[test]
fn test_norm_zero_vector() {
    let v = vec![0.0, 0.0, 0.0];
    assert!(norm_native(&v).abs() < EPSILON);
}

#[test]
fn test_norm_unit_vector() {
    let v = vec![1.0, 0.0, 0.0];
    assert!((norm_native(&v) - 1.0).abs() < EPSILON);
}

#[test]
fn test_norm_known_value() {
    let v = vec![3.0, 4.0];
    assert!((norm_native(&v) - 5.0).abs() < EPSILON);
}

// --- squared_l2_native tests ---

#[test]
fn test_squared_l2_identical() {
    let v = vec![1.0, 2.0, 3.0];
    assert!(squared_l2_native(&v, &v).abs() < EPSILON);
}

#[test]
fn test_squared_l2_known_value() {
    let a = vec![0.0, 0.0];
    let b = vec![3.0, 4.0];
    assert!((squared_l2_native(&a, &b) - 25.0).abs() < EPSILON);
}

// --- hamming_distance_fast tests ---

#[test]
fn test_hamming_identical() {
    let a = vec![1.0, 0.0, 1.0, 0.0];
    assert!(hamming_distance_native(&a, &a).abs() < EPSILON);
}

#[test]
fn test_hamming_all_different() {
    let a = vec![1.0, 0.0, 1.0, 0.0];
    let b = vec![0.0, 1.0, 0.0, 1.0];
    assert!((hamming_distance_native(&a, &b) - 4.0).abs() < EPSILON);
}

#[test]
fn test_hamming_partial() {
    let a = vec![1.0, 1.0, 0.0, 0.0];
    let b = vec![1.0, 0.0, 0.0, 1.0];
    assert!((hamming_distance_native(&a, &b) - 2.0).abs() < EPSILON);
}

#[test]
fn test_hamming_odd_dimension() {
    let a = vec![1.0, 0.0, 1.0, 0.0, 1.0];
    let b = vec![0.0, 0.0, 1.0, 1.0, 1.0];
    assert!((hamming_distance_native(&a, &b) - 2.0).abs() < EPSILON);
}

// --- jaccard_similarity_fast tests ---

#[test]
fn test_jaccard_identical() {
    let a = vec![1.0, 0.0, 1.0, 0.0];
    assert!((jaccard_similarity_native(&a, &a) - 1.0).abs() < EPSILON);
}

#[test]
fn test_jaccard_disjoint() {
    let a = vec![1.0, 0.0, 0.0, 0.0];
    let b = vec![0.0, 1.0, 0.0, 0.0];
    assert!(jaccard_similarity_native(&a, &b).abs() < EPSILON);
}

#[test]
fn test_jaccard_half_overlap() {
    let a = vec![1.0, 1.0, 0.0, 0.0];
    let b = vec![1.0, 0.0, 1.0, 0.0];
    // Intersection: 1, Union: 3
    assert!((jaccard_similarity_native(&a, &b) - (1.0 / 3.0)).abs() < EPSILON);
}

#[test]
fn test_jaccard_empty_sets() {
    let a = vec![0.0, 0.0, 0.0, 0.0];
    let b = vec![0.0, 0.0, 0.0, 0.0];
    assert!((jaccard_similarity_native(&a, &b) - 1.0).abs() < EPSILON);
}

// -------------------------------------------------------------------------
// TDD: Jaccard SIMD optimization tests (P2)
// -------------------------------------------------------------------------

#[test]
fn test_jaccard_simd_large_vectors() {
    // Test with 768D vectors (typical embedding size)
    let a: Vec<f32> = (0..768)
        .map(|i| if i % 2 == 0 { 1.0 } else { 0.0 })
        .collect();
    let b: Vec<f32> = (0..768)
        .map(|i| if i % 3 == 0 { 1.0 } else { 0.0 })
        .collect();

    let result = jaccard_similarity_native(&a, &b);

    // Verify result is in valid range
    assert!((0.0..=1.0).contains(&result), "Jaccard must be in [0,1]");
}

#[test]
fn test_jaccard_simd_aligned_vectors() {
    // Test with 8-aligned dimension (optimal for SIMD)
    let a: Vec<f32> = (0..64).map(|i| if i < 32 { 1.0 } else { 0.0 }).collect();
    let b: Vec<f32> = (0..64).map(|i| if i < 48 { 1.0 } else { 0.0 }).collect();

    let result = jaccard_similarity_native(&a, &b);

    // Intersection: 32 (first 32 elements), Union: 48
    let expected = 32.0 / 48.0;
    assert!(
        (result - expected).abs() < EPSILON,
        "Expected {expected}, got {result}"
    );
}

#[test]
fn test_jaccard_simd_unaligned_vectors() {
    // Test with non-8-aligned dimension (tests remainder handling)
    let a: Vec<f32> = (0..67).map(|i| if i < 30 { 1.0 } else { 0.0 }).collect();
    let b: Vec<f32> = (0..67).map(|i| if i < 40 { 1.0 } else { 0.0 }).collect();

    let result = jaccard_similarity_native(&a, &b);

    // Intersection: 30, Union: 40
    let expected = 30.0 / 40.0;
    assert!(
        (result - expected).abs() < EPSILON,
        "Expected {expected}, got {result}"
    );
}

#[test]
fn test_jaccard_consistency_scalar_vs_reference() {
    // Property test: verify consistency across different vector sizes
    for dim in [7, 8, 15, 16, 31, 32, 63, 64, 127, 128, 255, 256, 768] {
        let a: Vec<f32> = (0..dim)
            .map(|i| if (i * 7) % 11 < 6 { 1.0 } else { 0.0 })
            .collect();
        let b: Vec<f32> = (0..dim)
            .map(|i| if (i * 5) % 9 < 5 { 1.0 } else { 0.0 })
            .collect();

        let result = jaccard_similarity_native(&a, &b);

        // Compute reference manually
        let mut intersection = 0u32;
        let mut union = 0u32;
        for i in 0..dim {
            let in_a = a[i] > 0.5;
            let in_b = b[i] > 0.5;
            if in_a && in_b {
                intersection += 1;
            }
            if in_a || in_b {
                union += 1;
            }
        }
        let expected = if union == 0 {
            1.0
        } else {
            intersection as f32 / union as f32
        };

        assert!(
            (result - expected).abs() < EPSILON,
            "Dim {dim}: expected {expected}, got {result}"
        );
    }
}

// =========================================================================
// QW-2: Prefetch Helper Tests
// =========================================================================

#[test]
fn test_calculate_prefetch_distance_small_vectors() {
    // 32D vectors (128 bytes) -> raw = 2, clamped to 4
    assert_eq!(calculate_prefetch_distance(32), 4);
    // 64D vectors (256 bytes) -> raw = 4
    assert_eq!(calculate_prefetch_distance(64), 4);
}

#[test]
fn test_calculate_prefetch_distance_medium_vectors() {
    // 128D vectors (512 bytes) -> raw = 8
    assert_eq!(calculate_prefetch_distance(128), 8);
    // 256D vectors (1024 bytes) -> raw = 16
    assert_eq!(calculate_prefetch_distance(256), 16);
}

#[test]
fn test_calculate_prefetch_distance_large_vectors() {
    // 768D vectors (3072 bytes) -> raw = 48, clamped to 16
    assert_eq!(calculate_prefetch_distance(768), 16);
    // 1536D vectors (6144 bytes) -> raw = 96, clamped to 16
    assert_eq!(calculate_prefetch_distance(1536), 16);
}

#[test]
fn test_prefetch_vector_does_not_panic() {
    // Prefetch should never panic, even with edge cases
    let empty: Vec<f32> = vec![];
    prefetch_vector(&empty); // Empty vector - should be no-op

    let small = vec![1.0, 2.0, 3.0];
    prefetch_vector(&small); // Small vector

    let large = generate_test_vector(768, 0.0);
    prefetch_vector(&large); // Large vector
}

#[test]
fn test_l2_cache_line_constant() {
    // Verify constant is set correctly for x86_64
    assert_eq!(L2_CACHE_LINE_BYTES, 64);
}
