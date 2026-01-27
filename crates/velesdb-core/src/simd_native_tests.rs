//! Tests for `simd_native` module - Native SIMD operations.
//!
//! Separated from main module per project rules (tests in separate files).

use crate::simd_native::{
    batch_dot_product_native, cosine_normalized_native, cosine_similarity_fast,
    cosine_similarity_native, dot_product_native, euclidean_native, fast_rsqrt, simd_level,
    squared_l2_native, SimdLevel,
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
