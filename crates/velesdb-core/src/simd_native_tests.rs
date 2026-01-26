//! Tests for `simd_native` module - Native SIMD operations.

use crate::simd_native::{
    batch_dot_product_native, cosine_normalized_native, dot_product_native, simd_level,
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
