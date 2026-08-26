#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::float_cmp
)]
//! Tests for fused cosine similarity (EPIC-052/US-002)
//!
//! Tests that fused cosine similarity computes dot + norms in a single pass.

use super::cosine_similarity_native;

// Tolerance for f32 SIMD vs scalar comparison.
//
// `1e-4` is `simd_native_tests::SIMD_ABS_TOLERANCE`, the constant the rest of
// the SIMD suite already uses; the previous `5e-3` was 500x looser than these
// kernels need, which cost detection on the main loop for nothing.
//
// Tightening it does NOT make this test tail-sensitive, and no tolerance
// would: every size below is an exact multiple of its kernel's stride (512
// and 768 of the 4-acc's 64; 1024 of the 8-acc's 128), so `end_main ==
// end_ptr` and the masked remainder branch never executes. Injecting a
// dropped tail element into `x86_avx512.rs` leaves this test green at 5e-3
// and at 1e-4 alike. Tail coverage is `cosine_tail_tests.rs`, which picks
// dimensions that actually reach the remainder.
const EPSILON: f32 = 1e-4;

// ============================================================================
// Fused Cosine Tests
// ============================================================================

#[test]
fn test_fused_cosine_correctness() {
    // Test various vector sizes for correctness
    for size in [16, 32, 64, 128, 256, 384, 512, 768, 1024] {
        let a: Vec<f32> = (0..size).map(|i| ((i * 7) % 100) as f32 * 0.01).collect();
        let b: Vec<f32> = (0..size)
            .map(|i| (((size - i) * 13) % 100) as f32 * 0.01)
            .collect();

        let result = cosine_similarity_native(&a, &b);

        // Compute expected with scalar reference
        let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
        let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
        let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
        let expected = if norm_a > 0.0 && norm_b > 0.0 {
            (dot / (norm_a * norm_b)).clamp(-1.0, 1.0)
        } else {
            0.0
        };

        assert!(
            (result - expected).abs() < EPSILON,
            "Fused cosine failed for size {}: got {}, expected {}",
            size,
            result,
            expected
        );
    }
}

#[test]
fn test_fused_cosine_identical_vectors() {
    // Identical vectors should have cosine = 1.0
    for size in [16, 128, 768] {
        let a: Vec<f32> = (0..size).map(|i| ((i % 10) as f32) * 0.1).collect();
        let result = cosine_similarity_native(&a, &a);

        assert!(
            (result - 1.0).abs() < EPSILON,
            "Cosine of identical vectors should be 1.0, got {} for size {}",
            result,
            size
        );
    }
}

#[test]
fn test_fused_cosine_orthogonal_vectors() {
    // Orthogonal vectors should have cosine ≈ 0.0
    let a: Vec<f32> = vec![1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
    let b: Vec<f32> = vec![0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
    let result = cosine_similarity_native(&a, &b);

    assert!(
        result.abs() < EPSILON,
        "Cosine of orthogonal vectors should be ~0.0, got {}",
        result
    );
}

#[test]
fn test_fused_cosine_zero_vectors() {
    // Zero vectors should return 0.0
    let zeros = vec![0.0f32; 128];
    let ones = vec![1.0f32; 128];

    let result1 = cosine_similarity_native(&zeros, &ones);
    let result2 = cosine_similarity_native(&zeros, &zeros);

    assert!(
        result1.abs() < 1e-6,
        "Cosine with zero vector should be 0.0"
    );
    assert!(
        result2.abs() < 1e-6,
        "Cosine of two zero vectors should be 0.0"
    );
}

#[test]
fn test_fused_cosine_small_magnitude_vectors() {
    // Regression (#2103): the SIMD finish step guarded on the *product* of
    // squared norms (< EPSILON²), zeroing out legitimate vectors with norms
    // around 3.4e-4 while the scalar fallback returned the true cosine. The
    // result then flipped between 1.0 and 0.0 with dimension and CPU features.
    for magnitude in [1e-3f32, 1e-4, 1e-5] {
        // Cover the scalar fallback (dim < 4/8) and every SIMD kernel width,
        // including tail-exercising odd dims.
        for size in [3usize, 4, 7, 8, 15, 16, 17, 64, 100, 768] {
            let a: Vec<f32> = vec![magnitude; size];
            let result = cosine_similarity_native(&a, &a);
            assert!(
                (result - 1.0).abs() < 1e-3,
                "identical vectors of magnitude {magnitude} at dim {size} \
                 must have cosine ~1.0, got {result}"
            );
        }
    }

    // Anti-parallel small vectors must stay -1.0, not collapse to 0.0.
    let a = vec![1e-4f32; 16];
    let b: Vec<f32> = a.iter().map(|x| -x).collect();
    let result = cosine_similarity_native(&a, &b);
    assert!(
        (result + 1.0).abs() < 1e-3,
        "opposite small-magnitude vectors must have cosine ~-1.0, got {result}"
    );
}

#[test]
fn test_fused_cosine_opposite_vectors() {
    // Opposite vectors should have cosine = -1.0
    let a: Vec<f32> = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
    let b: Vec<f32> = a.iter().map(|x| -x).collect();

    let result = cosine_similarity_native(&a, &b);

    assert!(
        (result - (-1.0)).abs() < EPSILON,
        "Cosine of opposite vectors should be -1.0, got {}",
        result
    );
}

#[test]
#[ignore = "performance test - run with --ignored or PERF_TESTS=1"]
fn test_fused_cosine_performance() {
    // Verify performance is acceptable for 768D
    // This is a smoke test - actual benchmarks in benches/
    let size = 768;
    let a: Vec<f32> = (0..size).map(|i| ((i * 7) % 100) as f32 * 0.01).collect();
    let b: Vec<f32> = (0..size)
        .map(|i| (((size - i) * 13) % 100) as f32 * 0.01)
        .collect();

    // Warmup
    for _ in 0..100 {
        let _ = cosine_similarity_native(&a, &b);
    }

    // Measure
    let start = std::time::Instant::now();
    for _ in 0..1000 {
        let _ = cosine_similarity_native(&a, &b);
    }
    let elapsed = start.elapsed();
    let avg_ns = elapsed.as_nanos() as f64 / 1000.0;

    // Should be < 200ns per call on CI (allowing for slower CI runners)
    // Target < 35ns with Harley-Seal when optimized
    assert!(
        avg_ns < 200.0,
        "Cosine similarity too slow: {:.2}ns per call (target < 35ns with Harley-Seal, < 200ns CI)",
        avg_ns
    );
}
