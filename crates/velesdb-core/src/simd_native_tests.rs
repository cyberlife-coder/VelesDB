//! Tests for `simd_native` module - Native SIMD operations.
//!
//! Separated from main module per project rules (tests in separate files).

use crate::simd_native::{
    batch_dot_product_native, cosine_normalized_native, cosine_similarity_fast,
    cosine_similarity_native, dot_product_native, euclidean_native, fast_rsqrt,
    hamming_distance_native, jaccard_similarity_native, simd_level, squared_l2_native, SimdLevel,
};

#[test]
fn test_simd_level_cached() {
    // First call initializes the cache
    let level1 = simd_level();
    // Second call should return the same cached value
    let level2 = simd_level();

    assert_eq!(level1, level2, "SIMD level should be consistent");

    // Verify it's a valid level
    match level1 {
        SimdLevel::Avx512 | SimdLevel::Avx2 | SimdLevel::Neon | SimdLevel::Scalar => {}
    }
}

#[allow(clippy::cast_precision_loss)]
#[test]
fn test_dot_product_native_basic() {
    let a = vec![1.0, 2.0, 3.0, 4.0];
    let b = vec![5.0, 6.0, 7.0, 8.0];
    let result = dot_product_native(&a, &b);
    let expected: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    assert!((result - expected).abs() < 1e-5);
}

#[allow(clippy::cast_precision_loss)]
#[test]
fn test_dot_product_native_large() {
    let a: Vec<f32> = (0..768).map(|i| i as f32 * 0.001).collect();
    let b: Vec<f32> = (0..768).map(|i| (768 - i) as f32 * 0.001).collect();
    let result = dot_product_native(&a, &b);
    let expected: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    assert!(
        (result - expected).abs() < 0.01,
        "result={result}, expected={expected}"
    );
}

#[test]
fn test_squared_l2_native_basic() {
    let a = vec![1.0, 2.0, 3.0, 4.0];
    let b = vec![5.0, 6.0, 7.0, 8.0];
    let result = squared_l2_native(&a, &b);
    let expected: f32 = a
        .iter()
        .zip(b.iter())
        .map(|(x, y)| {
            let d = x - y;
            d * d
        })
        .sum();
    assert!((result - expected).abs() < 1e-5);
}

#[allow(clippy::cast_precision_loss)]
#[test]
fn test_squared_l2_native_large() {
    let a: Vec<f32> = (0..768).map(|i| i as f32 * 0.001).collect();
    let b: Vec<f32> = (0..768).map(|i| (768 - i) as f32 * 0.001).collect();
    let result = squared_l2_native(&a, &b);
    let expected: f32 = a
        .iter()
        .zip(b.iter())
        .map(|(x, y)| {
            let d = x - y;
            d * d
        })
        .sum();
    assert!(
        (result - expected).abs() < 0.01,
        "result={result}, expected={expected}"
    );
}

#[test]
fn test_cosine_normalized_native() {
    // Create unit vectors
    let a = vec![0.6, 0.8, 0.0, 0.0];
    let b = vec![1.0, 0.0, 0.0, 0.0];
    let result = cosine_normalized_native(&a, &b);
    assert!((result - 0.6).abs() < 1e-5);
}

#[test]
fn test_batch_dot_product_native() {
    let query = vec![1.0, 2.0, 3.0, 4.0];
    let candidates: Vec<Vec<f32>> = vec![
        vec![1.0, 0.0, 0.0, 0.0],
        vec![0.0, 1.0, 0.0, 0.0],
        vec![0.0, 0.0, 1.0, 0.0],
        vec![0.0, 0.0, 0.0, 1.0],
    ];
    let refs: Vec<&[f32]> = candidates.iter().map(Vec::as_slice).collect();

    let results = batch_dot_product_native(&refs, &query);
    assert_eq!(results.len(), 4);
    assert!((results[0] - 1.0).abs() < 1e-5);
    assert!((results[1] - 2.0).abs() < 1e-5);
    assert!((results[2] - 3.0).abs() < 1e-5);
    assert!((results[3] - 4.0).abs() < 1e-5);
}

// =========================================================================
// Additional Tests (migrated from inline)
// =========================================================================

#[test]
fn test_simd_level_detection() {
    let level = simd_level();
    assert!(matches!(
        level,
        SimdLevel::Avx512 | SimdLevel::Avx2 | SimdLevel::Neon | SimdLevel::Scalar
    ));
}

#[test]
fn test_simd_level_debug() {
    let level = simd_level();
    let debug = format!("{level:?}");
    assert!(!debug.is_empty());
}

#[test]
fn test_dot_product_native_zeros() {
    let a = vec![0.0; 16];
    let b = vec![1.0; 16];
    let result = dot_product_native(&a, &b);
    assert!((result - 0.0).abs() < 1e-5);
}

#[test]
fn test_dot_product_native_ones() {
    let a = vec![1.0; 32];
    let b = vec![1.0; 32];
    let result = dot_product_native(&a, &b);
    assert!((result - 32.0).abs() < 1e-5);
}

#[test]
fn test_dot_product_native_remainder() {
    let a: Vec<f32> = (0..19).map(|i| i as f32).collect();
    let b: Vec<f32> = (0..19).map(|_| 1.0).collect();
    let result = dot_product_native(&a, &b);
    let expected: f32 = (0..19).map(|i| i as f32).sum();
    assert!((result - expected).abs() < 1e-5);
}

#[test]
#[should_panic(expected = "Vector dimensions must match")]
fn test_dot_product_native_length_mismatch() {
    let a = vec![1.0, 2.0, 3.0];
    let b = vec![1.0, 2.0];
    let _ = dot_product_native(&a, &b);
}

#[test]
fn test_squared_l2_native_identical() {
    let a = vec![1.0, 2.0, 3.0, 4.0];
    let result = squared_l2_native(&a, &a);
    assert!((result - 0.0).abs() < 1e-5);
}

#[test]
#[should_panic(expected = "Vector dimensions must match")]
fn test_squared_l2_native_length_mismatch() {
    let a = vec![1.0, 2.0];
    let b = vec![1.0];
    let _ = squared_l2_native(&a, &b);
}

#[test]
fn test_euclidean_native_basic() {
    let a = vec![0.0, 0.0, 0.0];
    let b = vec![3.0, 4.0, 0.0];
    let result = euclidean_native(&a, &b);
    assert!((result - 5.0).abs() < 1e-5);
}

#[test]
fn test_euclidean_native_identical() {
    let a = vec![1.0, 2.0, 3.0, 4.0];
    let result = euclidean_native(&a, &a);
    assert!((result - 0.0).abs() < 1e-5);
}

#[test]
fn test_cosine_normalized_native_orthogonal() {
    let a = vec![1.0, 0.0, 0.0];
    let b = vec![0.0, 1.0, 0.0];
    let result = cosine_normalized_native(&a, &b);
    assert!((result - 0.0).abs() < 1e-5);
}

#[test]
fn test_cosine_similarity_native_identical() {
    let a = vec![1.0, 2.0, 3.0];
    let result = cosine_similarity_native(&a, &a);
    assert!((result - 1.0).abs() < 1e-5);
}

#[test]
fn test_cosine_similarity_native_opposite() {
    let a = vec![1.0, 2.0, 3.0];
    let b = vec![-1.0, -2.0, -3.0];
    let result = cosine_similarity_native(&a, &b);
    assert!((result - (-1.0)).abs() < 1e-5);
}

#[test]
fn test_cosine_similarity_native_zero_norm() {
    let a = vec![0.0, 0.0, 0.0];
    let b = vec![1.0, 2.0, 3.0];
    let result = cosine_similarity_native(&a, &b);
    assert!((result - 0.0).abs() < 1e-5);
}

#[test]
fn test_batch_dot_product_native_empty() {
    let query = vec![1.0, 2.0, 3.0];
    let candidates: Vec<&[f32]> = vec![];
    let results = batch_dot_product_native(&candidates, &query);
    assert!(results.is_empty());
}

#[test]
fn test_empty_vectors() {
    let a: Vec<f32> = vec![];
    let b: Vec<f32> = vec![];
    let result = dot_product_native(&a, &b);
    assert!((result - 0.0).abs() < 1e-5);
}

#[test]
fn test_single_element() {
    let a = vec![3.0];
    let b = vec![4.0];
    let result = dot_product_native(&a, &b);
    assert!((result - 12.0).abs() < 1e-5);
}

#[test]
fn test_exact_simd_width() {
    let a = vec![1.0; 16];
    let b = vec![1.0; 16];
    let result = dot_product_native(&a, &b);
    assert!((result - 16.0).abs() < 1e-5);
}

#[allow(clippy::cast_precision_loss)]
#[test]
fn test_high_dimension_384() {
    let a: Vec<f32> = (0..384).map(|i| (i as f32) / 384.0).collect();
    let b: Vec<f32> = (0..384).map(|i| (i as f32) / 384.0).collect();
    let result = dot_product_native(&a, &b);
    let expected: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    assert!((result - expected).abs() < 1e-3);
}

#[allow(clippy::cast_precision_loss)]
#[test]
fn test_high_dimension_1536() {
    let a: Vec<f32> = (0..1536).map(|i| (i as f32) / 1536.0).collect();
    let b: Vec<f32> = (0..1536).map(|i| ((i as f32) / 1536.0) * 0.5).collect();
    let result = dot_product_native(&a, &b);
    let expected: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    assert!((result - expected).abs() < 1e-2);
}

// =========================================================================
// Newton-Raphson Fast Inverse Square Root Tests (EPIC-PERF-001)
// =========================================================================

#[test]
fn test_fast_rsqrt_basic() {
    let result = fast_rsqrt(4.0);
    assert!(
        (result - 0.5).abs() < 0.01,
        "rsqrt(4) should be ~0.5, got {}",
        result
    );
}

#[test]
fn test_fast_rsqrt_one() {
    let result = fast_rsqrt(1.0);
    assert!(
        (result - 1.0).abs() < 0.01,
        "rsqrt(1) should be ~1.0, got {}",
        result
    );
}

#[test]
fn test_fast_rsqrt_accuracy() {
    for &x in &[0.25, 0.5, 1.0, 2.0, 4.0, 16.0, 100.0] {
        let fast = fast_rsqrt(x);
        let exact = 1.0 / x.sqrt();
        let rel_error = (fast - exact).abs() / exact;
        assert!(
            rel_error < 0.02,
            "rsqrt({}) rel_error {} > 2%",
            x,
            rel_error
        );
    }
}

#[test]
fn test_fast_rsqrt_vs_std() {
    let values: Vec<f32> = (1..100).map(|i| i as f32 * 0.1).collect();
    for x in values {
        let fast = fast_rsqrt(x);
        let std = 1.0 / x.sqrt();
        let rel_error = (fast - std).abs() / std;
        assert!(
            rel_error < 0.02,
            "rsqrt({}) rel_error {} > 2%",
            x,
            rel_error
        );
    }
}

#[test]
fn test_cosine_fast_uses_rsqrt() {
    let a = vec![1.0, 0.0, 0.0];
    let b = vec![1.0, 0.0, 0.0];
    let result = cosine_similarity_fast(&a, &b);
    assert!(
        (result - 1.0).abs() < 0.02,
        "parallel vectors should have cosine ~1.0"
    );

    let c = vec![1.0, 0.0, 0.0];
    let d = vec![0.0, 1.0, 0.0];
    let result2 = cosine_similarity_fast(&c, &d);
    assert!(
        result2.abs() < 0.02,
        "orthogonal vectors should have cosine ~0.0"
    );
}

#[test]
fn test_cosine_fast_normalized_vectors() {
    let a = vec![0.6, 0.8, 0.0];
    let b = vec![0.8, 0.6, 0.0];
    let result = cosine_similarity_fast(&a, &b);
    let expected = 0.6 * 0.8 + 0.8 * 0.6;
    assert!(
        (result - expected).abs() < 0.02,
        "cosine mismatch: {} vs {}",
        result,
        expected
    );
}

// =========================================================================
// Masked Load Tests - Eliminating Tail Loops (EPIC-PERF-002)
// =========================================================================

#[allow(clippy::cast_precision_loss)]
#[test]
fn test_dot_product_remainder_accuracy() {
    for len in [17, 19, 23, 31, 33, 47, 63, 65] {
        let a: Vec<f32> = (0..len).map(|i| (i as f32) * 0.1).collect();
        let b: Vec<f32> = (0..len).map(|i| (i as f32) * 0.1).collect();
        let result = dot_product_native(&a, &b);
        let expected: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
        let rel_error = if expected.abs() > 1e-6 {
            (result - expected).abs() / expected.abs()
        } else {
            (result - expected).abs()
        };
        assert!(rel_error < 1e-4, "len={} error={}", len, rel_error);
    }
}

#[allow(clippy::cast_precision_loss)]
#[test]
fn test_squared_l2_remainder_accuracy() {
    for len in [17, 19, 23, 31, 33] {
        let a: Vec<f32> = (0..len).map(|i| (i as f32) * 0.1).collect();
        let b: Vec<f32> = (0..len).map(|i| (i as f32) * 0.1 + 0.5).collect();
        let result = squared_l2_native(&a, &b);
        let expected: f32 = a
            .iter()
            .zip(b.iter())
            .map(|(x, y)| {
                let d = x - y;
                d * d
            })
            .sum();
        let rel_error = (result - expected).abs() / expected.abs();
        assert!(rel_error < 1e-4, "len={} error={}", len, rel_error);
    }
}

#[test]
fn test_dot_product_small_vectors_no_simd() {
    for len in [1, 2, 3, 4, 5, 7, 8, 15] {
        let a: Vec<f32> = (0..len).map(|i| (i + 1) as f32).collect();
        let b: Vec<f32> = vec![1.0; len];
        let result = dot_product_native(&a, &b);
        let expected: f32 = (1..=len).map(|i| i as f32).sum();
        assert!((result - expected).abs() < 1e-5, "len={} mismatch", len);
    }
}

// =========================================================================
// AVX-512 4-Accumulator Optimization Tests (EPIC-PERF-003)
// Tests verify dot_product_native dispatch to 4-acc for len >= 512
// =========================================================================

#[test]
fn test_dot_product_threshold_512_boundary() {
    // Test exact threshold boundary: 511 uses standard, 512+ uses 4-acc
    for len in [511usize, 512, 513, 575, 576, 577] {
        let a: Vec<f32> = (0..len).map(|i| (i as f32) * 0.001).collect();
        let b: Vec<f32> = (0..len).map(|i| ((len - 1 - i) as f32) * 0.001).collect();

        let simd_result = dot_product_native(&a, &b);
        let scalar_result: f32 = a.iter().zip(&b).map(|(x, y)| x * y).sum();

        let rel_error = if scalar_result.abs() > 1e-6 {
            (simd_result - scalar_result).abs() / scalar_result.abs()
        } else {
            (simd_result - scalar_result).abs()
        };

        assert!(
            rel_error < 1e-4,
            "Threshold test len={} failed: rel_error={} (simd={}, scalar={})",
            len,
            rel_error,
            simd_result,
            scalar_result
        );
    }
}

#[test]
fn test_dot_product_empty_vectors() {
    // len=0 should work without panic
    let a: Vec<f32> = vec![];
    let b: Vec<f32> = vec![];
    let result = dot_product_native(&a, &b);
    assert!(
        result.abs() < 1e-6,
        "Empty vectors should return ~0.0, got {}",
        result
    );
}

#[allow(clippy::cast_precision_loss)]
#[test]
fn test_dot_product_avx512_4acc_numerical_equivalence() {
    // Test numerical equivalence between SIMD and scalar
    // Expert recommendation: verify < 1e-5 divergence
    let a: Vec<f32> = (0..768).map(|i| i as f32 * 0.001).collect();
    let b: Vec<f32> = (0..768).map(|i| (768 - i) as f32 * 0.001).collect();

    let simd_result = dot_product_native(&a, &b);
    let scalar_result: f32 = a.iter().zip(&b).map(|(x, y)| x * y).sum();

    // SIMD floating point order differs from scalar - accept 1e-4 tolerance
    // Expert: "L'ordre des opérations flottantes change par rapport au scalaire (associativité)"
    assert!(
        (simd_result - scalar_result).abs() < 1e-4,
        "Numerical divergence: simd={}, scalar={}",
        simd_result,
        scalar_result
    );
}

#[allow(clippy::cast_precision_loss)]
#[test]
fn test_dot_product_avx512_4acc_large_vectors() {
    // Test with vectors >= 128 elements (4-acc threshold)
    for len in [128, 256, 512, 768, 1024, 1536] {
        let a: Vec<f32> = (0..len).map(|i| (i as f32) / (len as f32)).collect();
        let b: Vec<f32> = (0..len).map(|i| 1.0 - (i as f32) / (len as f32)).collect();

        let simd_result = dot_product_native(&a, &b);
        let scalar_result: f32 = a.iter().zip(&b).map(|(x, y)| x * y).sum();

        let rel_error = if scalar_result.abs() > 1e-6 {
            (simd_result - scalar_result).abs() / scalar_result.abs()
        } else {
            (simd_result - scalar_result).abs()
        };

        assert!(
            rel_error < 1e-4,
            "len={} rel_error={} (simd={}, scalar={})",
            len,
            rel_error,
            simd_result,
            scalar_result
        );
    }
}

#[allow(clippy::cast_precision_loss)]
#[test]
fn test_dot_product_remainder_bounds_elimination() {
    // Test remainder handling (len % 64 != 0) with bounds elimination
    // These lengths specifically test the scalar tail loop
    for len in [65, 66, 127, 129, 191, 193, 255, 257] {
        let a: Vec<f32> = (0..len).map(|i| i as f32).collect();
        let b: Vec<f32> = vec![1.0; len];

        let result = dot_product_native(&a, &b);
        let expected: f32 = (0..len).map(|i| i as f32).sum();

        assert!(
            (result - expected).abs() < 1e-4,
            "len={} mismatch: got={}, expected={}",
            len,
            result,
            expected
        );
    }
}

#[test]
fn test_dot_product_nan_propagation() {
    // NaN should propagate through SIMD operations
    let a = vec![f32::NAN, 1.0, 2.0, 3.0];
    let b = vec![1.0, 1.0, 1.0, 1.0];

    let result = dot_product_native(&a, &b);
    assert!(result.is_nan(), "NaN should propagate, got {}", result);
}

#[test]
fn test_dot_product_inf_handling() {
    // Infinity should be handled correctly
    let a = vec![f32::INFINITY, 1.0, 2.0, 3.0];
    let b = vec![0.5, 1.0, 1.0, 1.0];

    let result = dot_product_native(&a, &b);
    assert!(
        result.is_infinite(),
        "Infinity should propagate, got {}",
        result
    );
}

// =========================================================================
// Cosine Fused AVX-512 Tests (EPIC-PERF-004)
// TDD: These tests target the new cosine_avx512_fused function
// =========================================================================

#[test]
fn test_cosine_fused_identical_vectors() {
    let a = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
    let result = cosine_similarity_native(&a, &a);
    assert!(
        (result - 1.0).abs() < 1e-5,
        "Identical vectors should have cosine=1.0, got {}",
        result
    );
}

#[test]
fn test_cosine_fused_opposite_vectors() {
    let a = vec![1.0, 2.0, 3.0, 4.0];
    let b = vec![-1.0, -2.0, -3.0, -4.0];
    let result = cosine_similarity_native(&a, &b);
    assert!(
        (result - (-1.0)).abs() < 1e-5,
        "Opposite vectors should have cosine=-1.0, got {}",
        result
    );
}

#[test]
fn test_cosine_fused_orthogonal_vectors() {
    let a = vec![1.0, 0.0, 0.0, 0.0];
    let b = vec![0.0, 1.0, 0.0, 0.0];
    let result = cosine_similarity_native(&a, &b);
    assert!(
        result.abs() < 1e-5,
        "Orthogonal vectors should have cosine=0.0, got {}",
        result
    );
}

#[allow(clippy::cast_precision_loss)]
#[test]
fn test_cosine_fused_large_vectors_precision() {
    // Test precision with large vectors (fused dot+norms)
    let a: Vec<f32> = (0..768).map(|i| (i as f32) / 768.0).collect();
    let b: Vec<f32> = (0..768).map(|i| 1.0 - (i as f32) / 768.0).collect();

    let simd_result = cosine_similarity_native(&a, &b);

    // Compute scalar reference
    let dot: f32 = a.iter().zip(&b).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    let scalar_result = dot / (norm_a * norm_b);

    assert!(
        (simd_result - scalar_result).abs() < 0.02,
        "Cosine precision: simd={}, scalar={}",
        simd_result,
        scalar_result
    );
}

#[test]
fn test_cosine_fused_zero_vector() {
    let a = vec![0.0, 0.0, 0.0, 0.0];
    let b = vec![1.0, 2.0, 3.0, 4.0];
    let result = cosine_similarity_native(&a, &b);
    assert!(
        result.abs() < 1e-5,
        "Zero vector should give cosine=0.0, got {}",
        result
    );
}

#[test]
fn test_cosine_result_clamped() {
    // Ensure result is always in [-1, 1] even with floating point errors
    let a: Vec<f32> = (0..100).map(|i| (i as f32) * 0.01).collect();
    let b = a.clone();
    let result = cosine_similarity_native(&a, &b);
    assert!(
        (-1.0..=1.0).contains(&result),
        "Cosine should be clamped to [-1, 1], got {}",
        result
    );
}

// =========================================================================
// Jaccard Similarity Tests
// =========================================================================

#[test]
fn test_jaccard_scalar_identical_vectors() {
    let a = vec![1.0, 2.0, 3.0, 4.0];
    let result = jaccard_similarity_native(&a, &a);
    assert!(
        (result - 1.0).abs() < 1e-5,
        "Identical vectors should have Jaccard=1.0, got {}",
        result
    );
}

#[test]
fn test_jaccard_scalar_disjoint_vectors() {
    let a = vec![1.0, 0.0, 0.0, 0.0];
    let b = vec![0.0, 1.0, 0.0, 0.0];
    let result = jaccard_similarity_native(&a, &b);
    // intersection = 0, union = 2, Jaccard = 0/2 = 0
    assert!(
        result.abs() < 1e-5,
        "Disjoint vectors should have Jaccard=0.0, got {}",
        result
    );
}

#[test]
fn test_jaccard_scalar_partial_overlap() {
    let a = vec![1.0, 2.0, 0.0, 0.0];
    let b = vec![1.0, 0.0, 3.0, 0.0];
    // intersection = min(1,1) + min(2,0) + min(0,3) + min(0,0) = 1 + 0 + 0 + 0 = 1
    // union = max(1,1) + max(2,0) + max(0,3) + max(0,0) = 1 + 2 + 3 + 0 = 6
    // Jaccard = 1/6 = 0.166...
    let result = jaccard_similarity_native(&a, &b);
    let expected = 1.0 / 6.0;
    assert!(
        (result - expected).abs() < 1e-5,
        "Jaccard should be ~0.1667, got {}",
        result
    );
}

#[test]
fn test_jaccard_scalar_empty_union() {
    let a = vec![0.0, 0.0, 0.0, 0.0];
    let b = vec![0.0, 0.0, 0.0, 0.0];
    let result = jaccard_similarity_native(&a, &b);
    assert!(
        (result - 1.0).abs() < 1e-5,
        "Empty union should return Jaccard=1.0, got {}",
        result
    );
}

#[test]
fn test_jaccard_simd_small_vector() {
    // Vector < 8 elements should use scalar path
    let a = vec![1.0, 2.0, 3.0, 4.0];
    let b = vec![2.0, 1.0, 4.0, 3.0];
    let result = jaccard_similarity_native(&a, &b);
    // intersection = 1+1+3+3 = 8, union = 2+2+4+4 = 12, Jaccard = 8/12 = 0.666...
    let expected = 8.0 / 12.0;
    assert!(
        (result - expected).abs() < 1e-5,
        "Jaccard should be ~0.6667, got {}",
        result
    );
}

#[allow(clippy::cast_precision_loss)]
#[test]
fn test_jaccard_simd_matches_scalar() {
    // Test various sizes to trigger different SIMD paths
    for len in [4, 8, 16, 24, 32, 64, 128] {
        let a: Vec<f32> = (0..len).map(|i| ((i % 5) as f32) + 1.0).collect();
        let b: Vec<f32> = (0..len).map(|i| ((i % 3) as f32) + 0.5).collect();

        let simd_result = jaccard_similarity_native(&a, &b);

        // Compute scalar reference
        let scalar_inter: f32 = a.iter().zip(&b).map(|(x, y)| x.min(*y)).sum();
        let scalar_union: f32 = a.iter().zip(&b).map(|(x, y)| x.max(*y)).sum();
        let scalar_result = if scalar_union == 0.0 {
            1.0
        } else {
            scalar_inter / scalar_union
        };

        assert!(
            (simd_result - scalar_result).abs() < 1e-4,
            "len={}: SIMD result {} != scalar result {}",
            len,
            simd_result,
            scalar_result
        );
    }
}

// =========================================================================
// Hamming Distance Tests
// =========================================================================

#[test]
fn test_hamming_scalar_identical_vectors() {
    let a = vec![1.0, 0.0, 1.0, 0.0];
    let result = hamming_distance_native(&a, &a);
    assert!(
        (result - 0.0).abs() < 1e-5,
        "Identical vectors should have Hamming=0.0, got {}",
        result
    );
}

#[test]
fn test_hamming_scalar_completely_different() {
    let a = vec![1.0, 1.0, 1.0, 1.0];
    let b = vec![0.0, 0.0, 0.0, 0.0];
    let result = hamming_distance_native(&a, &b);
    // All 4 positions differ (1.0 > 0.5 vs 0.0 <= 0.5)
    assert!(
        (result - 4.0).abs() < 1e-5,
        "Completely different vectors should have Hamming=4.0, got {}",
        result
    );
}

#[test]
fn test_hamming_scalar_partial_differences() {
    let a = vec![0.8, 0.3, 0.9, 0.1]; // binary: 1, 0, 1, 0
    let b = vec![0.6, 0.7, 0.2, 0.4]; // binary: 1, 1, 0, 0
    let result = hamming_distance_native(&a, &b);
    // Positions 1 and 2 differ
    assert!(
        (result - 2.0).abs() < 1e-5,
        "Should have 2 differences, got {}",
        result
    );
}

#[test]
fn test_hamming_scalar_threshold_behavior() {
    // Test values around the 0.5 threshold
    // Position 3: 0.5001 > 0.5 is true, but 0.4999 > 0.5 is false → they differ
    let a = vec![0.51, 0.49, 0.5, 0.5001]; // binary: 1, 0, 0, 1
    let b = vec![0.52, 0.48, 0.5, 0.4999]; // binary: 1, 0, 0, 0
    let result = hamming_distance_native(&a, &b);
    // Only position 3 differs (0.5001 > 0.5 vs 0.4999 <= 0.5)
    assert!(
        (result - 1.0).abs() < 1e-5,
        "Should have 1 difference at position 3, got {}",
        result
    );
}

#[test]
fn test_hamming_simd_small_vector() {
    // Vector < 8 elements should use scalar path
    let a = vec![1.0, 0.0, 1.0, 0.0];
    let b = vec![0.0, 1.0, 1.0, 0.0];
    let result = hamming_distance_native(&a, &b);
    // Positions 0 and 1 differ
    assert!(
        (result - 2.0).abs() < 1e-5,
        "Should have 2 differences, got {}",
        result
    );
}

#[allow(clippy::cast_precision_loss)]
#[test]
fn test_hamming_simd_matches_scalar() {
    // Test various sizes to trigger different SIMD paths
    for len in [4, 8, 16, 24, 32, 64, 128] {
        let a: Vec<f32> = (0..len)
            .map(|i| if i % 3 == 0 { 0.8 } else { 0.2 })
            .collect();
        let b: Vec<f32> = (0..len)
            .map(|i| if i % 2 == 0 { 0.9 } else { 0.1 })
            .collect();

        let simd_result = hamming_distance_native(&a, &b);

        // Compute scalar reference
        let scalar_result: f32 = a
            .iter()
            .zip(&b)
            .filter(|(&x, &y)| (x > 0.5) != (y > 0.5))
            .count() as f32;

        assert!(
            (simd_result - scalar_result).abs() < 1e-4,
            "len={}: SIMD result {} != scalar result {}",
            len,
            simd_result,
            scalar_result
        );
    }
}

#[test]
#[should_panic(expected = "Vector length mismatch")]
fn test_jaccard_length_mismatch() {
    let a = vec![1.0, 2.0, 3.0];
    let b = vec![1.0, 2.0];
    let _ = jaccard_similarity_native(&a, &b);
}

#[test]
#[should_panic(expected = "Vector length mismatch")]
fn test_hamming_length_mismatch() {
    let a = vec![1.0, 2.0, 3.0];
    let b = vec![1.0, 2.0];
    let _ = hamming_distance_native(&a, &b);
}
