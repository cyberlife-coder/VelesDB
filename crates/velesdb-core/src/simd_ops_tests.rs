//! Tests for the simd_ops module.
//!
//! These tests verify the adaptive dispatch mechanism and correctness
//! of all SIMD operations across different backends.

use crate::distance::DistanceMetric;
use crate::simd_ops::{
    dispatch_info, distance, dot_product, force_rebenchmark, init_dispatch, log_dispatch_info,
    norm, normalize_inplace, similarity, SimdBackend,
};

#[test]
fn test_dispatch_table_initialization() {
    let info = dispatch_info();
    assert!(info.init_time_ms > 0.0, "Init time should be positive");
    assert!(
        info.init_time_ms < 30000.0,
        "Init time should be < 30 seconds"
    );
}

#[test]
fn test_dispatch_info_structure() {
    let info = dispatch_info();
    assert_eq!(info.dimensions, [128, 384, 768, 1024, 1536, 3072]);
    assert!(info.init_time_ms >= 0.0);

    // Verify backends are valid
    for backend in &info.dot_product_backends {
        assert!(matches!(
            backend,
            SimdBackend::NativeAvx512
                | SimdBackend::NativeAvx2
                | SimdBackend::NativeNeon
                | SimdBackend::Wide32
                | SimdBackend::Wide8
                | SimdBackend::Scalar
        ));
    }
}

#[test]
fn test_similarity_cosine_identical() {
    let a = vec![1.0, 0.0, 0.0];
    let b = vec![1.0, 0.0, 0.0];
    let sim = similarity(DistanceMetric::Cosine, &a, &b);
    assert!(
        (sim - 1.0).abs() < 1e-5,
        "Identical vectors should have cosine 1.0, got {}",
        sim
    );
}

#[test]
fn test_similarity_cosine_orthogonal() {
    let a = vec![1.0, 0.0, 0.0];
    let b = vec![0.0, 1.0, 0.0];
    let sim = similarity(DistanceMetric::Cosine, &a, &b);
    assert!(
        sim.abs() < 1e-5,
        "Orthogonal vectors should have cosine 0.0, got {}",
        sim
    );
}

#[test]
fn test_similarity_cosine_opposite() {
    let a = vec![1.0, 0.0, 0.0];
    let b = vec![-1.0, 0.0, 0.0];
    let sim = similarity(DistanceMetric::Cosine, &a, &b);
    assert!(
        (sim + 1.0).abs() < 1e-5,
        "Opposite vectors should have cosine -1.0, got {}",
        sim
    );
}

#[test]
fn test_similarity_euclidean() {
    let a = vec![0.0, 0.0, 0.0];
    let b = vec![3.0, 4.0, 0.0];
    let dist = similarity(DistanceMetric::Euclidean, &a, &b);
    assert!(
        (dist - 5.0).abs() < 1e-5,
        "Euclidean distance should be 5.0, got {}",
        dist
    );
}

#[test]
fn test_similarity_dot_product() {
    let a = vec![1.0, 2.0, 3.0];
    let b = vec![4.0, 5.0, 6.0];
    let dot = similarity(DistanceMetric::DotProduct, &a, &b);
    // 1*4 + 2*5 + 3*6 = 4 + 10 + 18 = 32
    assert!(
        (dot - 32.0).abs() < 1e-5,
        "Dot product should be 32.0, got {}",
        dot
    );
}

#[test]
fn test_similarity_hamming() {
    let a = vec![1.0, 0.0, 1.0, 0.0];
    let b = vec![1.0, 1.0, 0.0, 0.0];
    let ham = similarity(DistanceMetric::Hamming, &a, &b);
    // Positions 1 and 2 differ
    assert!(
        (ham - 2.0).abs() < 1e-5,
        "Hamming distance should be 2.0, got {}",
        ham
    );
}

#[test]
fn test_similarity_jaccard() {
    let a = vec![1.0, 1.0, 0.0, 0.0];
    let b = vec![1.0, 0.0, 1.0, 0.0];
    let jac = similarity(DistanceMetric::Jaccard, &a, &b);
    // intersection = min(1,1) + min(1,0) + min(0,1) + min(0,0) = 1
    // union = max(1,1) + max(1,0) + max(0,1) + max(0,0) = 1 + 1 + 1 + 0 = 3
    // jaccard = 1/3 ≈ 0.333
    assert!(
        (jac - 1.0 / 3.0).abs() < 1e-5,
        "Jaccard should be ~0.333, got {}",
        jac
    );
}

#[test]
fn test_distance_euclidean() {
    let a = vec![0.0, 0.0];
    let b = vec![3.0, 4.0];
    let dist = distance(DistanceMetric::Euclidean, &a, &b);
    assert!(
        (dist - 5.0).abs() < 1e-5,
        "Euclidean distance should be 5.0, got {}",
        dist
    );
}

#[test]
fn test_distance_cosine() {
    let a = vec![1.0, 0.0];
    let b = vec![1.0, 0.0];
    let dist = distance(DistanceMetric::Cosine, &a, &b);
    // distance = 1 - similarity = 1 - 1 = 0
    assert!(
        dist.abs() < 1e-5,
        "Cosine distance for identical vectors should be 0.0, got {}",
        dist
    );
}

#[test]
fn test_norm() {
    let v = vec![3.0, 4.0];
    let n = norm(&v);
    assert!((n - 5.0).abs() < 1e-5, "Norm should be 5.0, got {}", n);
}

#[test]
fn test_norm_unit_vector() {
    let v = vec![1.0, 0.0, 0.0];
    let n = norm(&v);
    assert!((n - 1.0).abs() < 1e-5, "Norm of unit vector should be 1.0");
}

#[test]
fn test_norm_zero_vector() {
    let v = vec![0.0, 0.0, 0.0];
    let n = norm(&v);
    assert!(n.abs() < 1e-10, "Norm of zero vector should be 0.0");
}

#[test]
fn test_normalize_inplace() {
    let mut v = vec![3.0, 4.0];
    normalize_inplace(&mut v);
    assert!(
        (v[0] - 0.6).abs() < 1e-5,
        "v[0] should be 0.6, got {}",
        v[0]
    );
    assert!(
        (v[1] - 0.8).abs() < 1e-5,
        "v[1] should be 0.8, got {}",
        v[1]
    );

    // Verify norm is now 1.0
    let n = norm(&v);
    assert!(
        (n - 1.0).abs() < 1e-5,
        "Normalized vector should have norm 1.0, got {}",
        n
    );
}

#[test]
fn test_normalize_inplace_zero_vector() {
    let mut v = vec![0.0, 0.0, 0.0];
    normalize_inplace(&mut v);
    // Should not panic, vector remains zero
    assert!(v.iter().all(|&x| x == 0.0));
}

#[test]
fn test_dot_product() {
    let a = vec![1.0, 2.0, 3.0, 4.0];
    let b = vec![5.0, 6.0, 7.0, 8.0];
    let dot = dot_product(&a, &b);
    // 1*5 + 2*6 + 3*7 + 4*8 = 5 + 12 + 21 + 32 = 70
    assert!(
        (dot - 70.0).abs() < 1e-5,
        "Dot product should be 70.0, got {}",
        dot
    );
}

#[test]
fn test_large_vectors_128d() {
    let a: Vec<f32> = (0..128).map(|i| (i as f32 * 0.01).sin()).collect();
    let b: Vec<f32> = (0..128).map(|i| (i as f32 * 0.02).cos()).collect();

    let sim = similarity(DistanceMetric::Cosine, &a, &b);
    assert!(
        (-1.0..=1.0).contains(&sim),
        "Cosine should be in [-1, 1], got {}",
        sim
    );

    let dist = similarity(DistanceMetric::Euclidean, &a, &b);
    assert!(dist >= 0.0, "Euclidean distance should be non-negative");
}

#[test]
fn test_large_vectors_384d() {
    let a: Vec<f32> = (0..384).map(|i| (i as f32 * 0.01).sin()).collect();
    let b: Vec<f32> = (0..384).map(|i| (i as f32 * 0.02).cos()).collect();

    let sim = similarity(DistanceMetric::Cosine, &a, &b);
    assert!((-1.0..=1.0).contains(&sim));

    let n = norm(&a);
    assert!(n > 0.0);
}

#[test]
fn test_large_vectors_768d() {
    let a: Vec<f32> = (0..768).map(|i| (i as f32 * 0.01).sin()).collect();
    let b: Vec<f32> = (0..768).map(|i| (i as f32 * 0.02).cos()).collect();

    let sim = similarity(DistanceMetric::Cosine, &a, &b);
    assert!((-1.0..=1.0).contains(&sim));

    let dist = similarity(DistanceMetric::Euclidean, &a, &b);
    assert!(dist >= 0.0);

    let dot = similarity(DistanceMetric::DotProduct, &a, &b);
    // Dot product can be any value
    assert!(dot.is_finite());
}

#[test]
fn test_large_vectors_1536d() {
    let a: Vec<f32> = (0..1536).map(|i| (i as f32 * 0.01).sin()).collect();
    let b: Vec<f32> = (0..1536).map(|i| (i as f32 * 0.02).cos()).collect();

    let sim = similarity(DistanceMetric::Cosine, &a, &b);
    assert!((-1.0..=1.0).contains(&sim));
}

#[test]
fn test_large_vectors_3072d() {
    let a: Vec<f32> = (0..3072).map(|i| (i as f32 * 0.01).sin()).collect();
    let b: Vec<f32> = (0..3072).map(|i| (i as f32 * 0.02).cos()).collect();

    let sim = similarity(DistanceMetric::Cosine, &a, &b);
    assert!((-1.0..=1.0).contains(&sim));
}

#[test]
fn test_backend_display() {
    assert_eq!(format!("{}", SimdBackend::NativeAvx512), "AVX-512");
    assert_eq!(format!("{}", SimdBackend::NativeAvx2), "AVX2");
    assert_eq!(format!("{}", SimdBackend::NativeNeon), "NEON");
    assert_eq!(format!("{}", SimdBackend::Wide32), "Wide32");
    assert_eq!(format!("{}", SimdBackend::Wide8), "Wide8");
    assert_eq!(format!("{}", SimdBackend::Scalar), "Scalar");
}

#[test]
fn test_consistency_across_backends() {
    // Test that results are consistent regardless of which backend is selected
    let a: Vec<f32> = (0..768).map(|i| (i as f32 * 0.01).sin()).collect();
    let b: Vec<f32> = (0..768).map(|i| (i as f32 * 0.02).cos()).collect();

    // Get results from simd_ops (adaptive)
    let sim_adaptive = similarity(DistanceMetric::Cosine, &a, &b);

    // Compare with simd_explicit (known good implementation)
    let sim_explicit = crate::simd_explicit::cosine_similarity_simd(&a, &b);

    // Should be very close (within floating point tolerance)
    assert!(
        (sim_adaptive - sim_explicit).abs() < 1e-4,
        "Adaptive ({}) and explicit ({}) should match",
        sim_adaptive,
        sim_explicit
    );
}

// =============================================================================
// Tests for init_dispatch(), force_rebenchmark(), log_dispatch_info()
// =============================================================================

#[test]
fn test_init_dispatch() {
    let info = init_dispatch();
    assert!(info.init_time_ms >= 0.0);
    assert!(!info.available_backends.is_empty());
    assert_eq!(info.dimensions.len(), 6);
}

#[test]
fn test_force_rebenchmark() {
    let info = force_rebenchmark();
    assert!(info.init_time_ms > 0.0, "Rebenchmark should take time");
    assert!(!info.available_backends.is_empty());
}

#[test]
fn test_log_dispatch_info() {
    // Just verify it doesn't panic
    log_dispatch_info();
}

#[test]
fn test_dispatch_info_display() {
    let info = dispatch_info();
    let display = format!("{}", info);
    assert!(display.contains("SIMD Dispatch Info"));
    assert!(display.contains("Init time"));
    assert!(display.contains("Available backends"));
}

#[test]
fn test_dispatch_info_available_backends() {
    let info = dispatch_info();

    // Should always have at least Scalar, Wide8, Wide32
    assert!(info.available_backends.len() >= 3);
    assert!(info.available_backends.contains(&SimdBackend::Scalar));
    assert!(info.available_backends.contains(&SimdBackend::Wide8));
    assert!(info.available_backends.contains(&SimdBackend::Wide32));
}
