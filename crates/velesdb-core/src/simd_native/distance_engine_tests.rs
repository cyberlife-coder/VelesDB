#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::float_cmp,
    clippy::approx_constant
)]
//! Tests for `DistanceEngine` cached function pointer dispatch.

use super::dispatch::DistanceEngine;
use super::{cosine_similarity_native, dot_product_native, squared_l2_native};

// ---------------------------------------------------------------------------
// Construction & trait bounds
// ---------------------------------------------------------------------------

#[test]
fn test_distance_engine_is_send_sync_copy() {
    fn assert_send_sync_copy<T: Send + Sync + Copy>() {}
    assert_send_sync_copy::<DistanceEngine>();
}

#[test]
fn test_distance_engine_debug_impl() {
    let engine = DistanceEngine::new(128);
    let debug = format!("{engine:?}");
    assert!(debug.contains("DistanceEngine"));
    assert!(debug.contains("128"));
}

#[test]
fn test_distance_engine_dimension() {
    let engine = DistanceEngine::new(768);
    assert_eq!(engine.dimension(), 768);
}

// ---------------------------------------------------------------------------
// Correctness: DistanceEngine must match *_native() functions
// ---------------------------------------------------------------------------

fn generate_vector(dim: usize, seed: f32) -> Vec<f32> {
    (0..dim)
        .map(|i| {
            #[allow(clippy::cast_precision_loss)]
            let v = (seed + i as f32 * 0.1).sin();
            v
        })
        .collect()
}

#[test]
fn test_distance_engine_dot_product_matches_native_128() {
    let engine = DistanceEngine::new(128);
    let a = generate_vector(128, 0.0);
    let b = generate_vector(128, 1.0);
    let native = dot_product_native(&a, &b);
    let cached = engine.dot_product(&a, &b);
    assert!(
        (native - cached).abs() < 1e-5,
        "dot_product mismatch: native={native}, engine={cached}"
    );
}

#[test]
fn test_distance_engine_dot_product_matches_native_768() {
    let engine = DistanceEngine::new(768);
    let a = generate_vector(768, 0.0);
    let b = generate_vector(768, 1.0);
    let native = dot_product_native(&a, &b);
    let cached = engine.dot_product(&a, &b);
    assert!(
        (native - cached).abs() < 1e-4,
        "dot_product mismatch: native={native}, engine={cached}"
    );
}

#[test]
fn test_distance_engine_dot_product_matches_native_1536() {
    let engine = DistanceEngine::new(1536);
    let a = generate_vector(1536, 0.0);
    let b = generate_vector(1536, 1.0);
    let native = dot_product_native(&a, &b);
    let cached = engine.dot_product(&a, &b);
    assert!(
        (native - cached).abs() < 1e-3,
        "dot_product mismatch: native={native}, engine={cached}"
    );
}

#[test]
fn test_distance_engine_squared_l2_matches_native_128() {
    let engine = DistanceEngine::new(128);
    let a = generate_vector(128, 0.0);
    let b = generate_vector(128, 1.0);
    let native = squared_l2_native(&a, &b);
    let cached = engine.squared_l2(&a, &b);
    assert!(
        (native - cached).abs() < 1e-5,
        "squared_l2 mismatch: native={native}, engine={cached}"
    );
}

#[test]
fn test_distance_engine_squared_l2_matches_native_768() {
    let engine = DistanceEngine::new(768);
    let a = generate_vector(768, 0.0);
    let b = generate_vector(768, 1.0);
    let native = squared_l2_native(&a, &b);
    let cached = engine.squared_l2(&a, &b);
    assert!(
        (native - cached).abs() < 1e-4,
        "squared_l2 mismatch: native={native}, engine={cached}"
    );
}

#[test]
fn test_distance_engine_cosine_matches_native_128() {
    let engine = DistanceEngine::new(128);
    let a = generate_vector(128, 0.0);
    let b = generate_vector(128, 1.0);
    let native = cosine_similarity_native(&a, &b);
    let cached = engine.cosine_similarity(&a, &b);
    assert!(
        (native - cached).abs() < 1e-5,
        "cosine mismatch: native={native}, engine={cached}"
    );
}

#[test]
fn test_distance_engine_cosine_matches_native_768() {
    let engine = DistanceEngine::new(768);
    let a = generate_vector(768, 0.0);
    let b = generate_vector(768, 1.0);
    let native = cosine_similarity_native(&a, &b);
    let cached = engine.cosine_similarity(&a, &b);
    assert!(
        (native - cached).abs() < 1e-5,
        "cosine mismatch: native={native}, engine={cached}"
    );
}

#[test]
fn test_distance_engine_euclidean_matches_native() {
    let engine = DistanceEngine::new(384);
    let a = generate_vector(384, 0.0);
    let b = generate_vector(384, 1.0);
    let native = squared_l2_native(&a, &b).sqrt();
    let cached = engine.euclidean(&a, &b);
    assert!(
        (native - cached).abs() < 1e-5,
        "euclidean mismatch: native={native}, engine={cached}"
    );
}

// ---------------------------------------------------------------------------
// Edge cases
// ---------------------------------------------------------------------------

#[test]
fn test_distance_engine_small_dimension() {
    // dim=3 — below all SIMD thresholds, should use scalar
    let engine = DistanceEngine::new(3);
    let a = vec![1.0, 2.0, 3.0];
    let b = vec![4.0, 5.0, 6.0];
    let result = engine.dot_product(&a, &b);
    // 1*4 + 2*5 + 3*6 = 32
    assert!((result - 32.0).abs() < 1e-6);
}

#[test]
fn test_distance_engine_identical_vectors() {
    let engine = DistanceEngine::new(128);
    let a = generate_vector(128, 0.5);
    let l2 = engine.squared_l2(&a, &a);
    assert!(
        l2.abs() < 1e-6,
        "L2 of identical vectors should be ~0, got {l2}"
    );
}

#[test]
fn test_distance_engine_copy_semantics() {
    let engine = DistanceEngine::new(128);
    let engine2 = engine; // Copy
    let a = generate_vector(128, 0.0);
    let b = generate_vector(128, 1.0);
    // Both should work independently
    let r1 = engine.dot_product(&a, &b);
    let r2 = engine2.dot_product(&a, &b);
    assert!((r1 - r2).abs() < 1e-10);
}

#[test]
fn test_distance_engine_all_common_dimensions() {
    // Verify engine works for all common embedding dimensions
    for dim in [128, 256, 384, 512, 768, 1024, 1536, 3072] {
        let engine = DistanceEngine::new(dim);
        let a = generate_vector(dim, 0.0);
        let b = generate_vector(dim, 1.0);

        let dp_native = dot_product_native(&a, &b);
        let dp_engine = engine.dot_product(&a, &b);
        assert!(
            (dp_native - dp_engine).abs() < 1e-3,
            "dot_product mismatch at dim={dim}: native={dp_native}, engine={dp_engine}"
        );
    }
}

// ---------------------------------------------------------------------------
// Thread safety (compile-time check)
// ---------------------------------------------------------------------------

#[test]
fn test_distance_engine_cross_thread() {
    let engine = DistanceEngine::new(128);
    let a = generate_vector(128, 0.0);
    let b = generate_vector(128, 1.0);

    let handle = std::thread::spawn(move || engine.dot_product(&a, &b));

    let result = handle.join().expect("thread panicked");
    assert!(result.is_finite());
}

// ---------------------------------------------------------------------------
// Correctness: DistanceEngine hamming/jaccard must match *_native()
// ---------------------------------------------------------------------------

#[test]
fn test_engine_hamming_matches_native() {
    for dim in [8, 16, 32, 64, 128, 256, 512, 768, 1024, 1536] {
        let engine = super::dispatch::DistanceEngine::new(dim);
        let a: Vec<f32> = (0..dim)
            .map(|i| if i % 3 == 0 { 1.0 } else { 0.0 })
            .collect();
        let b: Vec<f32> = (0..dim)
            .map(|i| if i % 2 == 0 { 1.0 } else { 0.0 })
            .collect();
        let cached = engine.hamming(&a, &b);
        let native = super::dispatch::hamming_distance_native(&a, &b);
        assert_eq!(
            cached, native,
            "hamming mismatch at dim={dim}: cached={cached}, native={native}"
        );
    }
}

#[test]
fn test_engine_jaccard_matches_native() {
    for dim in [8, 16, 32, 64, 128, 256, 512, 768, 1024, 1536] {
        let engine = super::dispatch::DistanceEngine::new(dim);
        let a: Vec<f32> = (0..dim)
            .map(|i| if i < dim / 2 { 1.0 } else { 0.0 })
            .collect();
        let b: Vec<f32> = (0..dim)
            .map(|i| if i < dim * 3 / 4 { 1.0 } else { 0.0 })
            .collect();
        let cached = engine.jaccard(&a, &b);
        let native = super::dispatch::jaccard_similarity_native(&a, &b);
        assert!(
            (cached - native).abs() < 1e-6,
            "jaccard mismatch at dim={dim}: cached={cached}, native={native}"
        );
    }
}
