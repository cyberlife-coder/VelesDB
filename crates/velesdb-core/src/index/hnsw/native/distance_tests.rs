//! Tests for distance computation engines.
//!
//! Extracted from `distance.rs` for maintainability (04-05 module splitting).

use super::distance::*;
use crate::distance::DistanceMetric;

#[allow(clippy::cast_precision_loss)]
#[test]
fn test_cosine_identical_vectors() {
    let engine = CpuDistance::new(DistanceMetric::Cosine);
    let v = vec![1.0, 2.0, 3.0];
    let dist = engine.distance(&v, &v);
    assert!(
        dist.abs() < 1e-5,
        "Identical vectors should have distance ~0"
    );
}

#[test]
fn test_euclidean_known_distance() {
    let engine = CpuDistance::new(DistanceMetric::Euclidean);
    let a = vec![0.0, 0.0, 0.0];
    let b = vec![3.0, 4.0, 0.0];
    let dist = engine.distance(&a, &b);
    assert!((dist - 5.0).abs() < 1e-5, "3-4-5 triangle");
}

#[test]
fn test_simd_matches_scalar() {
    let cpu = CpuDistance::new(DistanceMetric::Cosine);
    let simd = SimdDistance::new(DistanceMetric::Cosine);

    #[allow(clippy::cast_precision_loss)]
    let a: Vec<f32> = (0..768).map(|i| (i as f32 * 0.01).sin()).collect();
    #[allow(clippy::cast_precision_loss)]
    let b: Vec<f32> = (0..768).map(|i| (i as f32 * 0.02).cos()).collect();

    let cpu_dist = cpu.distance(&a, &b);
    let simd_dist = simd.distance(&a, &b);

    assert!(
        (cpu_dist - simd_dist).abs() < 1e-4,
        "SIMD should match scalar: cpu={cpu_dist}, simd={simd_dist}"
    );
}

// =========================================================================
// TDD Tests for PERF-2: Hamming/Jaccard SIMD + batch_distance optimization
// =========================================================================

#[test]
fn test_simd_hamming_uses_simd_implementation() {
    let simd = SimdDistance::new(DistanceMetric::Hamming);

    // Binary-like vectors (0.0 or 1.0)
    let a: Vec<f32> = (0..64)
        .map(|i| if i % 2 == 0 { 1.0 } else { 0.0 })
        .collect();
    let b: Vec<f32> = (0..64)
        .map(|i| if i % 3 == 0 { 1.0 } else { 0.0 })
        .collect();

    let dist = simd.distance(&a, &b);

    // Verify result is reasonable (hamming distance between these patterns)
    assert!(dist >= 0.0, "Hamming distance must be non-negative");
    assert!(dist <= 64.0, "Hamming distance cannot exceed vector length");
}

#[test]
fn test_simd_jaccard_uses_simd_implementation() {
    let simd = SimdDistance::new(DistanceMetric::Jaccard);

    // Binary-like vectors for set similarity
    let a: Vec<f32> = (0..64).map(|i| if i < 32 { 1.0 } else { 0.0 }).collect();
    let b: Vec<f32> = (0..64).map(|i| if i < 48 { 1.0 } else { 0.0 }).collect();

    let dist = simd.distance(&a, &b);

    // Jaccard distance = 1 - similarity, should be in [0, 1]
    assert!(
        (0.0..=1.0).contains(&dist),
        "Jaccard distance must be in [0,1]"
    );

    // Intersection = 32, Union = 48, Similarity = 32/48 = 0.667, Distance = 0.333
    let expected = 1.0 - (32.0 / 48.0);
    assert!(
        (dist - expected).abs() < 1e-4,
        "Jaccard distance: expected {expected}, got {dist}"
    );
}

#[test]
fn test_simd_hamming_identical_vectors() {
    let simd = SimdDistance::new(DistanceMetric::Hamming);
    let v: Vec<f32> = (0..32)
        .map(|i| if i % 2 == 0 { 1.0 } else { 0.0 })
        .collect();

    let dist = simd.distance(&v, &v);
    assert!(
        dist.abs() < 1e-5,
        "Identical vectors should have distance 0"
    );
}

#[test]
fn test_simd_jaccard_identical_vectors() {
    let simd = SimdDistance::new(DistanceMetric::Jaccard);
    let v: Vec<f32> = (0..32)
        .map(|i| if i % 2 == 0 { 1.0 } else { 0.0 })
        .collect();

    let dist = simd.distance(&v, &v);
    assert!(
        dist.abs() < 1e-5,
        "Identical vectors should have distance 0"
    );
}

#[test]
#[allow(clippy::cast_precision_loss)]
fn test_batch_distance_with_prefetch() {
    let simd = SimdDistance::new(DistanceMetric::Cosine);

    let query: Vec<f32> = (0..768).map(|i| (i as f32 * 0.01).sin()).collect();
    let candidates: Vec<Vec<f32>> = (0..100)
        .map(|j| {
            (0..768)
                .map(|i| ((i + j * 10) as f32 * 0.01).cos())
                .collect()
        })
        .collect();

    let candidate_refs: Vec<&[f32]> = candidates.iter().map(Vec::as_slice).collect();

    let distances = simd.batch_distance(&query, &candidate_refs);

    assert_eq!(distances.len(), 100, "Should return 100 distances");

    // Verify all distances are valid (cosine distance in [0, 2])
    for (i, &d) in distances.iter().enumerate() {
        assert!((0.0..=2.0).contains(&d), "Distance {i} = {d} out of range");
    }
}

#[test]
#[allow(clippy::cast_precision_loss)]
fn test_batch_distance_consistency() {
    let simd = SimdDistance::new(DistanceMetric::Euclidean);

    let query: Vec<f32> = (0..128).map(|i| i as f32).collect();
    let candidates: Vec<Vec<f32>> = (0..20)
        .map(|j| (0..128).map(|i| (i + j) as f32).collect())
        .collect();

    let candidate_refs: Vec<&[f32]> = candidates.iter().map(Vec::as_slice).collect();

    // Batch distance
    let batch_distances = simd.batch_distance(&query, &candidate_refs);

    // Individual distances
    let individual_distances: Vec<f32> = candidate_refs
        .iter()
        .map(|c| simd.distance(&query, c))
        .collect();

    // Results should match exactly
    for (i, (batch, individual)) in batch_distances
        .iter()
        .zip(individual_distances.iter())
        .enumerate()
    {
        assert!(
            (batch - individual).abs() < 1e-6,
            "Mismatch at {i}: batch={batch}, individual={individual}"
        );
    }
}

#[test]
fn test_batch_distance_empty() {
    let simd = SimdDistance::new(DistanceMetric::Cosine);
    let query = vec![1.0, 2.0, 3.0];
    let candidates: Vec<&[f32]> = vec![];

    let distances = simd.batch_distance(&query, &candidates);
    assert!(distances.is_empty(), "Empty candidates should return empty");
}

// =========================================================================
// Tests for NativeSimdDistance (AVX-512/NEON intrinsics)
// =========================================================================

#[test]
fn test_native_simd_matches_simd() {
    let simd = SimdDistance::new(DistanceMetric::Cosine);
    let native = NativeSimdDistance::new(DistanceMetric::Cosine);

    #[allow(clippy::cast_precision_loss)]
    let a: Vec<f32> = (0..768).map(|i| (i as f32 * 0.01).sin()).collect();
    #[allow(clippy::cast_precision_loss)]
    let b: Vec<f32> = (0..768).map(|i| (i as f32 * 0.02).cos()).collect();

    let simd_dist = simd.distance(&a, &b);
    let native_dist = native.distance(&a, &b);

    assert!(
        (simd_dist - native_dist).abs() < 1e-3,
        "Native SIMD should match SIMD: simd={simd_dist}, native={native_dist}"
    );
}

#[test]
fn test_native_simd_euclidean() {
    let native = NativeSimdDistance::new(DistanceMetric::Euclidean);

    let a = vec![0.0, 0.0, 0.0, 0.0];
    let b = vec![3.0, 4.0, 0.0, 0.0];

    let dist = native.distance(&a, &b);
    assert!((dist - 5.0).abs() < 1e-5, "3-4-5 triangle: got {dist}");
}

#[test]
#[allow(clippy::cast_precision_loss)]
fn test_native_simd_dot_product() {
    let native = NativeSimdDistance::new(DistanceMetric::DotProduct);

    let a: Vec<f32> = (0..128).map(|i| i as f32 * 0.1).collect();
    let b: Vec<f32> = (0..128).map(|i| (128 - i) as f32 * 0.1).collect();

    let dist = native.distance(&a, &b);
    // DotProduct distance is negative dot product
    assert!(dist < 0.0, "DotProduct distance should be negative");
}

// =========================================================================
// Additional tests for 90% coverage
// =========================================================================

#[test]
fn test_cpu_distance_dot_product() {
    let cpu = CpuDistance::new(DistanceMetric::DotProduct);
    let a = vec![1.0, 2.0, 3.0];
    let b = vec![4.0, 5.0, 6.0];
    let dist = cpu.distance(&a, &b);
    // dot = 1*4 + 2*5 + 3*6 = 32, distance = -32
    assert!((dist + 32.0).abs() < 1e-5);
}

#[test]
fn test_cpu_distance_hamming() {
    let cpu = CpuDistance::new(DistanceMetric::Hamming);
    let a = vec![1.0, 0.0, 1.0, 0.0];
    let b = vec![1.0, 1.0, 0.0, 0.0];
    let dist = cpu.distance(&a, &b);
    // 2 bits differ (positions 1 and 2)
    assert!((dist - 2.0).abs() < 1e-5);
}

#[test]
fn test_cpu_distance_jaccard() {
    let cpu = CpuDistance::new(DistanceMetric::Jaccard);
    let a = vec![1.0, 1.0, 0.0, 0.0];
    let b = vec![1.0, 0.0, 1.0, 0.0];
    // intersection = min(1,1) + min(1,0) + min(0,1) + min(0,0) = 1
    // union = max(1,1) + max(1,0) + max(0,1) + max(0,0) = 3
    // similarity = 1/3, distance = 2/3
    let dist = cpu.distance(&a, &b);
    let expected = 1.0 - (1.0 / 3.0);
    assert!((dist - expected).abs() < 1e-5);
}

#[test]
fn test_cpu_distance_metric_accessor() {
    let cpu = CpuDistance::new(DistanceMetric::Euclidean);
    assert_eq!(cpu.metric(), DistanceMetric::Euclidean);
}

#[test]
fn test_simd_distance_metric_accessor() {
    let simd = SimdDistance::new(DistanceMetric::Cosine);
    assert_eq!(simd.metric(), DistanceMetric::Cosine);
}

#[test]
fn test_native_simd_metric_accessor() {
    let native = NativeSimdDistance::new(DistanceMetric::DotProduct);
    assert_eq!(native.metric(), DistanceMetric::DotProduct);
}

#[test]
fn test_simd_dot_product() {
    let simd = SimdDistance::new(DistanceMetric::DotProduct);
    let a = vec![1.0, 2.0, 3.0, 4.0];
    let b = vec![1.0, 1.0, 1.0, 1.0];
    let dist = simd.distance(&a, &b);
    // dot = 10, distance = -10
    assert!((dist + 10.0).abs() < 1e-4);
}

#[test]
fn test_simd_euclidean() {
    let simd = SimdDistance::new(DistanceMetric::Euclidean);
    let a = vec![0.0, 0.0, 0.0, 0.0];
    let b = vec![3.0, 4.0, 0.0, 0.0];
    let dist = simd.distance(&a, &b);
    assert!((dist - 5.0).abs() < 1e-4);
}

#[test]
fn test_native_simd_hamming() {
    let native = NativeSimdDistance::new(DistanceMetric::Hamming);
    let a: Vec<f32> = (0..32)
        .map(|i| if i % 2 == 0 { 1.0 } else { 0.0 })
        .collect();
    let b: Vec<f32> = (0..32)
        .map(|i| if i % 3 == 0 { 1.0 } else { 0.0 })
        .collect();
    let dist = native.distance(&a, &b);
    assert!(dist >= 0.0);
}

#[test]
fn test_native_simd_jaccard() {
    let native = NativeSimdDistance::new(DistanceMetric::Jaccard);
    let a = vec![1.0, 1.0, 0.0, 0.0];
    let b = vec![1.0, 1.0, 1.0, 0.0];
    let dist = native.distance(&a, &b);
    assert!((0.0..=1.0).contains(&dist));
}

// =========================================================================
// Tests for AdaptiveSimdDistance bug fix (returns distance, not similarity)
// =========================================================================

#[test]
fn test_adaptive_simd_cosine_returns_distance() {
    let adaptive = AdaptiveSimdDistance::new(DistanceMetric::Cosine);

    // Identical vectors should have distance ~0 (not similarity ~1)
    let v = vec![1.0, 2.0, 3.0, 4.0];
    let dist = adaptive.distance(&v, &v);
    assert!(
        dist.abs() < 1e-4,
        "AdaptiveSimdDistance should return distance ~0 for identical vectors, got {dist}"
    );

    // Opposite vectors should have distance ~2 (not similarity ~-1)
    let opposite: Vec<f32> = v.iter().map(|x| -x).collect();
    let dist_opposite = adaptive.distance(&v, &opposite);
    assert!(
        (dist_opposite - 2.0).abs() < 1e-4,
        "AdaptiveSimdDistance should return distance ~2 for opposite vectors, got {dist_opposite}"
    );
}

#[test]
fn test_adaptive_simd_dot_product_returns_distance() {
    let adaptive = AdaptiveSimdDistance::new(DistanceMetric::DotProduct);

    // Positive dot product should give negative distance (lower = better)
    let a = vec![1.0, 2.0, 3.0];
    let b = vec![1.0, 1.0, 1.0];
    let dist = adaptive.distance(&a, &b);
    // dot = 1*1 + 2*1 + 3*1 = 6, distance = -6
    assert!(
        dist < 0.0,
        "AdaptiveSimdDistance DotProduct should return negative distance, got {dist}"
    );
    assert!(
        (dist + 6.0).abs() < 1e-4,
        "Expected distance ~-6, got {dist}"
    );
}

#[test]
fn test_adaptive_simd_jaccard_returns_distance() {
    let adaptive = AdaptiveSimdDistance::new(DistanceMetric::Jaccard);

    // Identical vectors should have distance ~0
    let v: Vec<f32> = (0..32)
        .map(|i| if i % 2 == 0 { 1.0 } else { 0.0 })
        .collect();
    let dist = adaptive.distance(&v, &v);
    assert!(
        dist.abs() < 1e-4,
        "AdaptiveSimdDistance Jaccard should return distance ~0 for identical vectors, got {dist}"
    );

    // Jaccard distance should be in [0, 1]
    let b: Vec<f32> = (0..32)
        .map(|i| if i % 3 == 0 { 1.0 } else { 0.0 })
        .collect();
    let dist2 = adaptive.distance(&v, &b);
    assert!(
        (0.0..=1.0).contains(&dist2),
        "AdaptiveSimdDistance Jaccard distance should be in [0,1], got {dist2}"
    );
}

#[test]
fn test_adaptive_simd_matches_native_simd() {
    // Ensure AdaptiveSimdDistance returns same results as NativeSimdDistance
    let adaptive = AdaptiveSimdDistance::new(DistanceMetric::Cosine);
    let native = NativeSimdDistance::new(DistanceMetric::Cosine);

    #[allow(clippy::cast_precision_loss)]
    let a: Vec<f32> = (0..768).map(|i| (i as f32 * 0.01).sin()).collect();
    #[allow(clippy::cast_precision_loss)]
    let b: Vec<f32> = (0..768).map(|i| (i as f32 * 0.02).cos()).collect();

    let adaptive_dist = adaptive.distance(&a, &b);
    let native_dist = native.distance(&a, &b);

    assert!(
        (adaptive_dist - native_dist).abs() < 1e-3,
        "AdaptiveSimdDistance should match NativeSimdDistance: adaptive={adaptive_dist}, native={native_dist}"
    );
}

#[test]
fn test_adaptive_simd_euclidean_returns_distance() {
    let adaptive = AdaptiveSimdDistance::new(DistanceMetric::Euclidean);

    let a = vec![0.0, 0.0, 0.0, 0.0];
    let b = vec![3.0, 4.0, 0.0, 0.0];
    let dist = adaptive.distance(&a, &b);

    assert!(
        (dist - 5.0).abs() < 1e-4,
        "AdaptiveSimdDistance Euclidean should return 5.0 for 3-4-5 triangle, got {dist}"
    );
}

#[test]
fn test_adaptive_simd_hamming_returns_distance() {
    let adaptive = AdaptiveSimdDistance::new(DistanceMetric::Hamming);

    let a: Vec<f32> = (0..32)
        .map(|i| if i % 2 == 0 { 1.0 } else { 0.0 })
        .collect();
    let b: Vec<f32> = (0..32)
        .map(|i| if i % 3 == 0 { 1.0 } else { 0.0 })
        .collect();

    let dist = adaptive.distance(&a, &b);
    assert!(
        dist >= 0.0,
        "AdaptiveSimdDistance Hamming distance should be non-negative, got {dist}"
    );
}

#[test]
#[allow(clippy::cast_precision_loss)]
fn test_native_simd_batch_dot_product() {
    let native = NativeSimdDistance::new(DistanceMetric::DotProduct);
    let query: Vec<f32> = vec![1.0; 16];
    let candidates: Vec<Vec<f32>> = (0..5).map(|i| vec![(i + 1) as f32; 16]).collect();
    let candidate_refs: Vec<&[f32]> = candidates.iter().map(Vec::as_slice).collect();

    let distances = native.batch_distance(&query, &candidate_refs);
    assert_eq!(distances.len(), 5);
    // Each dot product = 16 * (i+1), distance = -dot
    for (i, &d) in distances.iter().enumerate() {
        let expected = -16.0 * ((i + 1) as f32);
        assert!(
            (d - expected).abs() < 1e-3,
            "i={i}: got {d}, expected {expected}"
        );
    }
}

#[test]
fn test_native_simd_batch_euclidean() {
    let native = NativeSimdDistance::new(DistanceMetric::Euclidean);
    let query = vec![0.0; 8];
    let candidates: Vec<Vec<f32>> = vec![vec![1.0; 8], vec![2.0; 8]];
    let candidate_refs: Vec<&[f32]> = candidates.iter().map(Vec::as_slice).collect();

    let distances = native.batch_distance(&query, &candidate_refs);
    assert_eq!(distances.len(), 2);
}

#[test]
fn test_cosine_scalar_zero_norm() {
    // Test division by zero case — scalar functions are pub(super) via module
    let engine = CpuDistance::new(DistanceMetric::Cosine);
    let a = vec![0.0, 0.0, 0.0];
    let b = vec![1.0, 2.0, 3.0];
    let dist = engine.distance(&a, &b);
    assert!(
        (dist - 1.0).abs() < 1e-5,
        "Zero norm should return distance 1.0"
    );
}

#[test]
fn test_jaccard_scalar_zero_union() {
    // Test division by zero case
    let engine = CpuDistance::new(DistanceMetric::Jaccard);
    let a = vec![0.0, 0.0, 0.0];
    let b = vec![0.0, 0.0, 0.0];
    let dist = engine.distance(&a, &b);
    assert!(
        (dist - 1.0).abs() < 1e-5,
        "Zero union should return distance 1.0"
    );
}

#[test]
fn test_cpu_batch_distance_default_impl() {
    let cpu = CpuDistance::new(DistanceMetric::Euclidean);
    let query = vec![0.0, 0.0, 0.0];
    let c1 = vec![1.0, 0.0, 0.0];
    let c2 = vec![0.0, 2.0, 0.0];
    let candidates: Vec<&[f32]> = vec![&c1, &c2];

    let distances = cpu.batch_distance(&query, &candidates);
    assert_eq!(distances.len(), 2);
    assert!((distances[0] - 1.0).abs() < 1e-5);
    assert!((distances[1] - 2.0).abs() < 1e-5);
}

#[test]
fn test_hamming_scalar_all_same() {
    let engine = CpuDistance::new(DistanceMetric::Hamming);
    let a = vec![1.0, 2.0, 3.0];
    let dist = engine.distance(&a, &a);
    assert!((dist - 0.0).abs() < 1e-5);
}

#[test]
fn test_hamming_scalar_all_different() {
    let engine = CpuDistance::new(DistanceMetric::Hamming);
    let a = vec![1.0, 2.0, 3.0];
    let b = vec![4.0, 5.0, 6.0];
    let dist = engine.distance(&a, &b);
    assert!((dist - 3.0).abs() < 1e-5);
}
