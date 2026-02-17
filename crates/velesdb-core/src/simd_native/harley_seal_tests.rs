#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::float_cmp
)]
//! Tests for Harley-Seal population count (EPIC-052/US-003)
//!
//! Tests that Harley-Seal AVX2 correctly computes population count for Hamming/Jaccard.

use super::{hamming_distance_native, jaccard_similarity_native};

// ============================================================================
// Harley-Seal Hamming Tests
// ============================================================================

#[test]
fn test_harley_seal_hamming_correctness() {
    // Test binary vectors with Harley-Seal
    // Vectors with values > 0.5 are considered "1", else "0"
    let a: Vec<f32> = vec![1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 1.0];
    let b: Vec<f32> = vec![1.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0];

    // Expected: positions 1, 2, 5, 6 differ = 4 differences
    let result = hamming_distance_native(&a, &b);
    let expected = 4.0f32;

    assert!(
        (result - expected).abs() < 1e-6,
        "Harley-Seal Hamming failed: got {result}, expected {expected}"
    );
}

#[test]
fn test_harley_seal_hamming_all_ones() {
    // All identical vectors should give 0
    for size in [32, 64, 128, 256, 512, 768] {
        let a: Vec<f32> = vec![1.0; size];
        let result = hamming_distance_native(&a, &a);
        assert!(
            result.abs() < 1e-6,
            "Hamming of identical vectors should be 0 for size {size}"
        );
    }
}

#[test]
fn test_harley_seal_hamming_all_zeros() {
    // All zeros vectors should give 0
    for size in [32, 64, 128, 256, 512, 768] {
        let a: Vec<f32> = vec![0.0; size];
        let result = hamming_distance_native(&a, &a);
        assert!(
            result.abs() < 1e-6,
            "Hamming of zero vectors should be 0 for size {size}"
        );
    }
}

#[test]
fn test_harley_seal_hamming_opposite() {
    // Completely opposite vectors
    let size = 256;
    let a: Vec<f32> = (0..size)
        .map(|i| if i % 2 == 0 { 1.0 } else { 0.0 })
        .collect();
    let b: Vec<f32> = (0..size)
        .map(|i| if i % 2 == 0 { 0.0 } else { 1.0 })
        .collect();

    let result = hamming_distance_native(&a, &b);
    let expected = size as f32;

    assert!(
        (result - expected).abs() < 1e-6,
        "Harley-Seal Hamming opposite failed: got {result}, expected {expected}"
    );
}

// ============================================================================
// Harley-Seal Jaccard Tests
// ============================================================================

#[test]
fn test_harley_seal_jaccard_correctness() {
    // Test with set-like vectors (30% density)
    let size = 100;
    // Sets A and B with known overlap
    let a: Vec<f32> = (0..size)
        .map(|i| if i < 30 { 1.0 } else { 0.0 }) // First 30 elements
        .collect();
    let b: Vec<f32> = (0..size)
        .map(|i| if (20..50).contains(&i) { 1.0 } else { 0.0 }) // 20-50 (30 elements, overlap 10)
        .collect();

    // Intersection = 10, Union = 50
    // Jaccard = intersection / union = 10/50 = 0.2
    let result = jaccard_similarity_native(&a, &b);
    let expected = 0.2f32;

    assert!(
        (result - expected).abs() < 1e-5,
        "Harley-Seal Jaccard failed: got {result}, expected {expected}"
    );
}

#[test]
fn test_harley_seal_jaccard_identical() {
    // Identical sets should have Jaccard = 1.0
    for size in [32, 64, 128, 256] {
        let a: Vec<f32> = (0..size)
            .map(|i| if i % 3 == 0 { 1.0 } else { 0.0 })
            .collect();
        let result = jaccard_similarity_native(&a, &a);

        assert!(
            (result - 1.0).abs() < 1e-6,
            "Jaccard of identical sets should be 1.0 for size {size}: got {result}"
        );
    }
}

#[test]
fn test_harley_seal_jaccard_disjoint() {
    // Disjoint sets should have Jaccard = 0.0
    let size = 100;
    let a: Vec<f32> = (0..size).map(|i| if i < 50 { 1.0 } else { 0.0 }).collect();
    let b: Vec<f32> = (0..size).map(|i| if i >= 50 { 1.0 } else { 0.0 }).collect();

    let result = jaccard_similarity_native(&a, &b);

    assert!(
        result.abs() < 1e-6,
        "Jaccard of disjoint sets should be 0.0: got {result}"
    );
}

#[test]
#[ignore = "performance test - run with --ignored or PERF_TESTS=1"]
fn test_harley_seal_jaccard_performance() {
    // Performance test for 768D
    let size = 768;
    let a: Vec<f32> = (0..size)
        .map(|i| if (i * 7) % 10 < 3 { 1.0 } else { 0.0 })
        .collect();
    let b: Vec<f32> = (0..size)
        .map(|i| if (i * 13) % 10 < 3 { 1.0 } else { 0.0 })
        .collect();

    // Warmup
    for _ in 0..100 {
        let _ = jaccard_similarity_native(&a, &b);
    }

    // Measure
    let start = std::time::Instant::now();
    for _ in 0..1000 {
        let _ = jaccard_similarity_native(&a, &b);
    }
    let elapsed = start.elapsed();
    let avg_ns = elapsed.as_nanos() as f64 / 1000.0;

    // Should be < 200ns per call on CI (allowing for slower CI runners)
    // Target < 35ns with Harley-Seal when optimized
    assert!(
        avg_ns < 200.0,
        "Jaccard similarity too slow: {avg_ns:.2}ns per call (target < 35ns with Harley-Seal, < 200ns CI)"
    );
}

// ============================================================================
// Comparison with Scalar Reference
// ============================================================================

#[test]
fn test_harley_seal_vs_scalar_hamming() {
    // Compare Harley-Seal with scalar reference
    for size in [32, 64, 128, 256, 512, 768] {
        let a: Vec<f32> = (0..size)
            .map(|i| if (i * 7) % 5 == 0 { 1.0 } else { 0.0 })
            .collect();
        let b: Vec<f32> = (0..size)
            .map(|i| if (i * 13) % 5 == 0 { 1.0 } else { 0.0 })
            .collect();

        let result = hamming_distance_native(&a, &b);

        // Scalar reference
        let expected = a
            .iter()
            .zip(b.iter())
            .filter(|(x, y)| (**x > 0.5) != (**y > 0.5))
            .count() as f32;

        assert!(
            (result - expected).abs() < 1e-6,
            "Harley-Seal vs scalar failed for size {size}: got {result}, expected {expected}"
        );
    }
}

#[test]
fn test_harley_seal_vs_scalar_jaccard() {
    // Compare Harley-Seal Jaccard with scalar reference
    for size in [32, 64, 128, 256, 512] {
        let a: Vec<f32> = (0..size)
            .map(|i| if (i * 7) % 5 == 0 { 1.0 } else { 0.0 })
            .collect();
        let b: Vec<f32> = (0..size)
            .map(|i| if (i * 13) % 5 == 0 { 1.0 } else { 0.0 })
            .collect();

        let result = jaccard_similarity_native(&a, &b);

        // Scalar reference
        let (intersection, union): (f32, f32) =
            a.iter()
                .zip(b.iter())
                .fold((0.0_f32, 0.0_f32), |(inter, uni), (x, y)| {
                    let x_bit: f32 = if *x > 0.5 { 1.0 } else { 0.0 };
                    let y_bit: f32 = if *y > 0.5 { 1.0 } else { 0.0 };
                    (inter + x_bit.min(y_bit), uni + x_bit.max(y_bit))
                });

        let expected = if union > 0.0 {
            intersection / union
        } else {
            0.0
        };

        assert!(
            (result - expected).abs() < 1e-5,
            "Harley-Seal Jaccard vs scalar failed for size {size}: got {result}, expected {expected}"
        );
    }
}
