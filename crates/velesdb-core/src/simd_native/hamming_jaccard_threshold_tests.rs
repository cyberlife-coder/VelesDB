#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::float_cmp
)]
//! Threshold-based f32 Hamming and Jaccard dispatch (#2106 item 17).
//!
//! This file was called `harley_seal_tests.rs` and its header claimed to cover
//! "threshold dispatch population count". **No threshold dispatch or carry-save-adder network
//! exists anywhere in this tree** — a sweep for `harley`, `carry.save` and `csa`
//! finds nothing but the name itself. Binary popcount, where it happens at all,
//! is `vcntq_u8`, `VPOPCNTDQ` or `u64::count_ones`, and all three are correct.
//!
//! What these tests actually exercise is the *threshold* form of the two
//! metrics: `hamming_distance_native` and `jaccard_similarity_native` over f32
//! vectors, where a component counts as a set bit when it exceeds 0.5. That is
//! a different algorithm from popcount over packed bits, and naming it after
//! one it does not use cost a reader the only cheap signal they had about which
//! code path a failure implicates.
//!
//! The name is now what the file does. `hamming_jaccard_tests.rs` next door
//! covers the same two entry points across dimension thresholds and batch
//! shapes; this one stays separate because it is about the 0.5 threshold
//! semantics rather than dispatch width.

use super::{hamming_distance_native, jaccard_similarity_native};

// ============================================================================
// Threshold Hamming Tests
// ============================================================================

#[test]
fn test_threshold_hamming_correctness() {
    // Binary-valued f32 vectors through the 0.5 threshold
    // Vectors with values > 0.5 are considered "1", else "0"
    let a: Vec<f32> = vec![1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 1.0];
    let b: Vec<f32> = vec![1.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0];

    // Expected: positions 1, 2, 5, 6 differ = 4 differences
    let result = hamming_distance_native(&a, &b);
    let expected = 4.0f32;

    assert!(
        (result - expected).abs() < 1e-6,
        "threshold dispatch Hamming failed: got {}, expected {}",
        result,
        expected
    );
}

#[test]
fn test_threshold_hamming_all_ones() {
    // All identical vectors should give 0
    for size in [32, 64, 128, 256, 512, 768] {
        let a: Vec<f32> = vec![1.0; size];
        let result = hamming_distance_native(&a, &a);
        assert!(
            result.abs() < 1e-6,
            "Hamming of identical vectors should be 0 for size {}",
            size
        );
    }
}

#[test]
fn test_threshold_hamming_all_zeros() {
    // All zeros vectors should give 0
    for size in [32, 64, 128, 256, 512, 768] {
        let a: Vec<f32> = vec![0.0; size];
        let result = hamming_distance_native(&a, &a);
        assert!(
            result.abs() < 1e-6,
            "Hamming of zero vectors should be 0 for size {}",
            size
        );
    }
}

#[test]
fn test_threshold_hamming_opposite() {
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
        "threshold dispatch Hamming opposite failed: got {}, expected {}",
        result,
        expected
    );
}

// ============================================================================
// Threshold Jaccard Tests
// ============================================================================

#[test]
fn test_threshold_jaccard_correctness() {
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
        "threshold dispatch Jaccard failed: got {}, expected {}",
        result,
        expected
    );
}

#[test]
fn test_threshold_jaccard_identical() {
    // Identical sets should have Jaccard = 1.0
    for size in [32, 64, 128, 256] {
        let a: Vec<f32> = (0..size)
            .map(|i| if i % 3 == 0 { 1.0 } else { 0.0 })
            .collect();
        let result = jaccard_similarity_native(&a, &a);

        assert!(
            (result - 1.0).abs() < 1e-6,
            "Jaccard of identical sets should be 1.0 for size {}: got {}",
            size,
            result
        );
    }
}

#[test]
fn test_threshold_jaccard_disjoint() {
    // Disjoint sets should have Jaccard = 0.0
    let size = 100;
    let a: Vec<f32> = (0..size).map(|i| if i < 50 { 1.0 } else { 0.0 }).collect();
    let b: Vec<f32> = (0..size).map(|i| if i >= 50 { 1.0 } else { 0.0 }).collect();

    let result = jaccard_similarity_native(&a, &b);

    assert!(
        result.abs() < 1e-6,
        "Jaccard of disjoint sets should be 0.0: got {}",
        result
    );
}

#[test]
#[ignore = "performance test - run with --ignored or PERF_TESTS=1"]
fn test_threshold_jaccard_performance() {
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
    // Target < 35ns with threshold dispatch when optimized
    assert!(
        avg_ns < 200.0,
        "Jaccard similarity too slow: {:.2}ns per call (target < 35ns with threshold dispatch, < 200ns CI)",
        avg_ns
    );
}

// ============================================================================
// Comparison with Scalar Reference
// ============================================================================

#[test]
fn test_threshold_vs_scalar_hamming() {
    // Compare threshold dispatch with scalar reference
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
            "threshold dispatch vs scalar failed for size {}: got {}, expected {}",
            size,
            result,
            expected
        );
    }
}

#[test]
fn test_threshold_vs_scalar_jaccard() {
    // Compare threshold dispatch Jaccard with scalar reference
    for size in [32, 64, 128, 256, 512] {
        let a: Vec<f32> = (0..size)
            .map(|i| if (i * 7) % 5 == 0 { 1.0 } else { 0.0 })
            .collect();
        // Use a different residue so b differs from a and a real (non-trivial)
        // intersection/union ratio is exercised instead of self-similarity.
        let b: Vec<f32> = (0..size)
            .map(|i| if (i * 13) % 5 == 1 { 1.0 } else { 0.0 })
            .collect();

        let result = jaccard_similarity_native(&a, &b);

        // Authoritative scalar reference (kept in sync with production).
        let expected = super::scalar::jaccard_scalar(&a, &b);

        assert!(
            (result - expected).abs() < 1e-6,
            "threshold dispatch Jaccard vs scalar failed for size {}: got {}, expected {}",
            size,
            result,
            expected
        );

        // Cover the zero-union guard path: production returns 1.0 for all-zero.
        let z = vec![0.0f32; size];
        assert!(
            (jaccard_similarity_native(&z, &z) - 1.0).abs() < 1e-6,
            "Zero-union Jaccard should be 1.0 for size {}",
            size
        );
    }
}
