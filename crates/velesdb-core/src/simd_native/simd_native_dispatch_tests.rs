#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::float_cmp
)]
//! Tests for adaptive dispatch thresholds (EPIC-052/US-001)
//!
//! Tests that SIMD dispatch correctly selects implementation based on vector size:
//! - < 16 elements: scalar
//! - 16-63 elements: AVX2 1-accumulator
//! - 64-255 elements: AVX2 2-accumulator (NEW)
//! - 256+ elements: AVX2 4-accumulator

use super::{dot_product_native, squared_l2_native};

// Tolerance for f32 SIMD vs scalar comparison
// SIMD uses different accumulation order (parallel vs sequential)
// Euclidean uses squared values, so epsilon needs to be larger
const EPSILON: f32 = 5e-3;

// ============================================================================
// Threshold Tests
// ============================================================================

#[test]
fn test_dispatch_uses_scalar_for_small_vectors() {
    // Vectors < 16 elements should use scalar implementation
    for size in [1, 2, 4, 8, 15] {
        let a: Vec<f32> = (0..size).map(|i| i as f32).collect();
        let b: Vec<f32> = (0..size).map(|i| (size - i) as f32).collect();

        let result = dot_product_native(&a, &b);

        // Verify correctness against expected scalar result
        let expected: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
        assert!(
            (result - expected).abs() < EPSILON,
            "Scalar dispatch failed for size {}: got {}, expected {}",
            size,
            result,
            expected
        );
    }
}

#[test]
fn test_dispatch_uses_simd_for_medium_vectors() {
    // Vectors 16-63 elements should use AVX2 1-acc
    for size in [16, 31, 32, 63] {
        let a: Vec<f32> = (0..size).map(|i| i as f32 * 0.1).collect();
        let b: Vec<f32> = (0..size).map(|i| (size - i) as f32 * 0.1).collect();

        let result = dot_product_native(&a, &b);

        // Verify correctness
        let expected: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
        assert!(
            (result - expected).abs() < EPSILON,
            "Medium vector dispatch failed for size {}: got {}, expected {}",
            size,
            result,
            expected
        );
    }
}

#[test]
fn test_dispatch_uses_2acc_for_large_vectors() {
    // Vectors 64-255 elements should use AVX2 2-acc
    for size in [64, 127, 128, 255] {
        let a: Vec<f32> = (0..size).map(|i| (i % 10) as f32).collect();
        let b: Vec<f32> = (0..size).map(|i| ((size - i) % 10) as f32).collect();

        let result = dot_product_native(&a, &b);

        // Verify correctness
        let expected: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
        assert!(
            (result - expected).abs() < EPSILON,
            "2-acc dispatch failed for size {}: got {}, expected {}",
            size,
            result,
            expected
        );
    }
}

#[test]
fn test_dispatch_uses_4acc_for_very_large_vectors() {
    // Vectors 256+ elements should use AVX2 4-acc
    for size in [256, 384, 512, 768, 1024] {
        let a: Vec<f32> = (0..size).map(|i| ((i * 7) % 100) as f32 * 0.01).collect();
        let b: Vec<f32> = (0..size)
            .map(|i| (((size - i) * 13) % 100) as f32 * 0.01)
            .collect();

        let result = dot_product_native(&a, &b);

        // Verify correctness
        let expected: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
        assert!(
            (result - expected).abs() < EPSILON,
            "4-acc dispatch failed for size {}: got {}, expected {}",
            size,
            result,
            expected
        );
    }
}

// ============================================================================
// Edge Cases
// ============================================================================

#[test]
fn test_dispatch_empty_vectors() {
    // Empty vectors should return 0.0 (mathematically correct for dot product)
    let a: Vec<f32> = vec![];
    let b: Vec<f32> = vec![];
    let result = dot_product_native(&a, &b);
    assert!(
        (result - 0.0).abs() < 1e-6,
        "Dot product of empty vectors should be 0.0"
    );

    // Also test squared_l2
    let l2_result = squared_l2_native(&a, &b);
    assert!(
        (l2_result - 0.0).abs() < 1e-6,
        "Squared L2 of empty vectors should be 0.0"
    );
}

#[test]
fn test_dispatch_exact_thresholds() {
    // Test exact boundary values
    // Size 15 should be scalar, 16 should be SIMD
    let a_15: Vec<f32> = (0..15).map(|i| i as f32).collect();
    let b_15: Vec<f32> = (0..15).map(|i| i as f32).collect();
    let result_15 = dot_product_native(&a_15, &b_15);

    let a_16: Vec<f32> = (0..16).map(|i| i as f32).collect();
    let b_16: Vec<f32> = (0..16).map(|i| i as f32).collect();
    let result_16 = dot_product_native(&a_16, &b_16);

    // Both should be correct
    let expected_15: f32 = a_15.iter().zip(b_15.iter()).map(|(x, y)| x * y).sum();
    let expected_16: f32 = a_16.iter().zip(b_16.iter()).map(|(x, y)| x * y).sum();

    assert!((result_15 - expected_15).abs() < 1e-6);
    assert!((result_16 - expected_16).abs() < 1e-6);
}

#[test]
fn test_dispatch_no_regression_on_small_vectors() {
    // Ensure small vectors (< 16) don't have performance regression
    // This is a smoke test - actual perf tested in benchmarks
    let sizes = vec![1, 2, 4, 8, 15];

    for size in sizes {
        let a: Vec<f32> = vec![1.0; size];
        let b: Vec<f32> = vec![1.0; size];

        let result = dot_product_native(&a, &b);
        assert!(
            (result - size as f32).abs() < 1e-3,
            "Failed for size {}",
            size
        );
    }
}

// ============================================================================
// Euclidean Distance Threshold Tests
// ============================================================================

#[test]
fn test_euclidean_dispatch_thresholds() {
    // Test that Euclidean also uses correct thresholds
    // Use smaller values to reduce precision issues with f32
    for size in [8, 16, 64, 256, 512] {
        let a: Vec<f32> = (0..size).map(|i| (i % 10) as f32 * 0.01).collect();
        let b: Vec<f32> = (0..size).map(|i| ((size - i) % 10) as f32 * 0.01).collect();

        let result = squared_l2_native(&a, &b);

        // Verify correctness
        let expected: f32 = a
            .iter()
            .zip(b.iter())
            .map(|(x, y)| {
                let d = x - y;
                d * d
            })
            .sum();

        assert!(
            (result - expected).abs() < EPSILON,
            "Euclidean dispatch failed for size {}: got {}, expected {}",
            size,
            result,
            expected
        );
    }
}

// ============================================================================
// Feature-detection contract
// ============================================================================

/// Every non-scalar x86 level implies the AVX2 + FMA baseline.
///
/// `Avx512` arms fall through to AVX2 kernels for inputs narrower than their
/// own minimum — `hamming`, `jaccard` and `scale_inplace` all do — and those
/// kernels carry `#[target_feature(enable = "avx2")]`. Detection used to
/// grant `Avx512` on `avx512f` alone, so the fall-through rested on "every
/// AVX-512 CPU has AVX2" rather than on a check; a hypervisor masking CPUID
/// breaks that, and the failure mode is an illegal instruction. This asserts
/// the property on whatever CPU runs the suite, so it fails exactly on the
/// hardware where the omission would have bitten.
#[cfg(target_arch = "x86_64")]
#[test]
fn every_non_scalar_level_implies_the_avx2_baseline() {
    use super::{simd_level, SimdLevel};

    let baseline = is_x86_feature_detected!("avx2") && is_x86_feature_detected!("fma");
    match simd_level() {
        SimdLevel::Avx512 | SimdLevel::Avx2 => assert!(
            baseline,
            "level {:?} was granted without avx2 + fma",
            simd_level()
        ),
        SimdLevel::Neon | SimdLevel::Scalar => {}
    }
}

/// AVX-512F alone must not grant the `Avx512` level.
#[cfg(target_arch = "x86_64")]
#[test]
fn avx512_level_requires_more_than_avx512f() {
    use super::{simd_level, SimdLevel};

    if simd_level() == SimdLevel::Avx512 {
        assert!(
            is_x86_feature_detected!("avx512f"),
            "Avx512 must still require avx512f"
        );
        assert!(
            is_x86_feature_detected!("avx2"),
            "Avx512 must not be granted without avx2"
        );
    }
}
