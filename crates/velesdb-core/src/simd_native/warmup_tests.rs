#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::float_cmp
)]
//! Tests for `OnceLock` warmup (EPIC-052/US-004)
//!
//! Tests that `warmup_simd_cache` correctly initializes SIMD caches.

use super::{cosine_similarity_native, dot_product_native, warmup_simd_cache};

// ============================================================================
// Warmup Tests
// ============================================================================

/// Test that warmup reduces latency (performance test, may be flaky in CI).
/// Run with: cargo test -- --ignored
#[test]
#[ignore = "performance test - run with --ignored or PERF_TESTS=1"]
fn test_warmup_reduces_first_request_latency() {
    // Call warmup
    warmup_simd_cache();

    // First request after warmup should be fast
    let size = 768;
    let a: Vec<f32> = (0..size).map(|i| ((i * 7) % 100) as f32 * 0.01).collect();
    let b: Vec<f32> = (0..size)
        .map(|i| (((size - i) * 13) % 100) as f32 * 0.01)
        .collect();

    // Measure first call after warmup
    let start = std::time::Instant::now();
    let _ = dot_product_native(&a, &b);
    let first_call_ns = start.elapsed().as_nanos() as f64;

    // Should be < 250ns (not cold start - cold start can be 2-3x slower)
    assert!(
        first_call_ns < 250.0,
        "First call after warmup too slow: {first_call_ns:.2}ns (should be < 250ns)"
    );

    // Verify correctness
    let result = dot_product_native(&a, &b);
    let expected: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    assert!(
        (result - expected).abs() < 1e-3,
        "Warmup affected correctness: got {result}, expected {expected}"
    );
}

#[test]
fn test_warmup_idempotent() {
    // Multiple warmups should be safe
    warmup_simd_cache();
    warmup_simd_cache();
    warmup_simd_cache();

    // Should still work correctly
    let a: Vec<f32> = vec![1.0, 2.0, 3.0, 4.0];
    let b: Vec<f32> = vec![5.0, 6.0, 7.0, 8.0];

    let result = dot_product_native(&a, &b);
    assert!(
        (result - 70.0).abs() < 1e-6,
        "Multiple warmups broke correctness"
    );
}

#[test]
fn test_warmup_all_functions() {
    warmup_simd_cache();

    // Test dot product
    let a: Vec<f32> = (0..768).map(|i| i as f32 * 0.01).collect();
    let b: Vec<f32> = (0..768).map(|i| (767 - i) as f32 * 0.01).collect();

    let dot_result = dot_product_native(&a, &b);
    assert!(dot_result > 0.0, "Dot product after warmup failed");

    // Test cosine
    let cos_result = cosine_similarity_native(&a, &b);
    assert!(
        (-1.0..=1.0).contains(&cos_result),
        "Cosine after warmup failed"
    );
}
