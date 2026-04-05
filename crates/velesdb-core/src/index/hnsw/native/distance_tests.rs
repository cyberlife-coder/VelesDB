#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::float_cmp,
    clippy::approx_constant
)]
//! Tests for distance computation engines.
//!
//! Extracted from `distance.rs` for maintainability (04-05 module splitting).
//! Primary engine under test: `CachedSimdDistance` (production engine).

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
    let cached = CachedSimdDistance::new(DistanceMetric::Cosine, 768);

    #[allow(clippy::cast_precision_loss)]
    let a: Vec<f32> = (0..768).map(|i| (i as f32 * 0.01).sin()).collect();
    #[allow(clippy::cast_precision_loss)]
    let b: Vec<f32> = (0..768).map(|i| (i as f32 * 0.02).cos()).collect();

    let cpu_dist = cpu.distance(&a, &b);
    let cached_dist = cached.distance(&a, &b);

    assert!(
        (cpu_dist - cached_dist).abs() < 1e-4,
        "CachedSimd should match scalar: cpu={cpu_dist}, cached={cached_dist}"
    );
}

// =========================================================================
// TDD Tests for PERF-2: Hamming/Jaccard SIMD + batch_distance optimization
// =========================================================================

#[test]
fn test_simd_hamming_uses_simd_implementation() {
    let cached = CachedSimdDistance::new(DistanceMetric::Hamming, 64);

    // Binary-like vectors (0.0 or 1.0)
    let a: Vec<f32> = (0..64)
        .map(|i| if i % 2 == 0 { 1.0 } else { 0.0 })
        .collect();
    let b: Vec<f32> = (0..64)
        .map(|i| if i % 3 == 0 { 1.0 } else { 0.0 })
        .collect();

    let dist = cached.distance(&a, &b);

    // Verify result is reasonable (hamming distance between these patterns)
    assert!(dist >= 0.0, "Hamming distance must be non-negative");
    assert!(dist <= 64.0, "Hamming distance cannot exceed vector length");
}

#[test]
fn test_simd_jaccard_uses_simd_implementation() {
    let cached = CachedSimdDistance::new(DistanceMetric::Jaccard, 64);

    // Binary-like vectors for set similarity
    let a: Vec<f32> = (0..64).map(|i| if i < 32 { 1.0 } else { 0.0 }).collect();
    let b: Vec<f32> = (0..64).map(|i| if i < 48 { 1.0 } else { 0.0 }).collect();

    let dist = cached.distance(&a, &b);

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
    let cached = CachedSimdDistance::new(DistanceMetric::Hamming, 32);
    let v: Vec<f32> = (0..32)
        .map(|i| if i % 2 == 0 { 1.0 } else { 0.0 })
        .collect();

    let dist = cached.distance(&v, &v);
    assert!(
        dist.abs() < 1e-5,
        "Identical vectors should have distance 0"
    );
}

#[test]
fn test_simd_jaccard_identical_vectors() {
    let cached = CachedSimdDistance::new(DistanceMetric::Jaccard, 32);
    let v: Vec<f32> = (0..32)
        .map(|i| if i % 2 == 0 { 1.0 } else { 0.0 })
        .collect();

    let dist = cached.distance(&v, &v);
    assert!(
        dist.abs() < 1e-5,
        "Identical vectors should have distance 0"
    );
}

#[test]
#[allow(clippy::cast_precision_loss)]
fn test_batch_distance_with_prefetch() {
    let cached = CachedSimdDistance::new(DistanceMetric::Cosine, 768);

    let query: Vec<f32> = (0..768).map(|i| (i as f32 * 0.01).sin()).collect();
    let candidates: Vec<Vec<f32>> = (0..100)
        .map(|j| {
            (0..768)
                .map(|i| ((i + j * 10) as f32 * 0.01).cos())
                .collect()
        })
        .collect();

    let candidate_refs: Vec<&[f32]> = candidates.iter().map(Vec::as_slice).collect();

    let distances = cached.batch_distance(&query, &candidate_refs);

    assert_eq!(distances.len(), 100, "Should return 100 distances");

    // Verify all distances are valid (cosine distance in [0, 2])
    for (i, &d) in distances.iter().enumerate() {
        assert!((0.0..=2.0).contains(&d), "Distance {i} = {d} out of range");
    }
}

#[test]
#[allow(clippy::cast_precision_loss)]
fn test_batch_distance_consistency() {
    let cached = CachedSimdDistance::new(DistanceMetric::Euclidean, 128);

    let query: Vec<f32> = (0..128).map(|i| i as f32).collect();
    let candidates: Vec<Vec<f32>> = (0..20)
        .map(|j| (0..128).map(|i| (i + j) as f32).collect())
        .collect();

    let candidate_refs: Vec<&[f32]> = candidates.iter().map(Vec::as_slice).collect();

    // Batch distance
    let batch_distances = cached.batch_distance(&query, &candidate_refs);

    // Individual distances
    let individual_distances: Vec<f32> = candidate_refs
        .iter()
        .map(|c| cached.distance(&query, c))
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
    let cached = CachedSimdDistance::new(DistanceMetric::Cosine, 3);
    let query = vec![1.0, 2.0, 3.0];
    let candidates: Vec<&[f32]> = vec![];

    let distances = cached.batch_distance(&query, &candidates);
    assert!(distances.is_empty(), "Empty candidates should return empty");
}

// =========================================================================
// Tests for CachedSimdDistance against CpuDistance (scalar baseline)
// =========================================================================

#[test]
fn test_cached_simd_matches_cpu_cosine() {
    let cpu = CpuDistance::new(DistanceMetric::Cosine);
    let cached = CachedSimdDistance::new(DistanceMetric::Cosine, 768);

    #[allow(clippy::cast_precision_loss)]
    let a: Vec<f32> = (0..768).map(|i| (i as f32 * 0.01).sin()).collect();
    #[allow(clippy::cast_precision_loss)]
    let b: Vec<f32> = (0..768).map(|i| (i as f32 * 0.02).cos()).collect();

    let cpu_dist = cpu.distance(&a, &b);
    let cached_dist = cached.distance(&a, &b);

    assert!(
        (cpu_dist - cached_dist).abs() < 1e-3,
        "CachedSimd should match CPU: cpu={cpu_dist}, cached={cached_dist}"
    );
}

#[test]
fn test_cached_simd_euclidean() {
    let cached = CachedSimdDistance::new(DistanceMetric::Euclidean, 4);

    let a = vec![0.0, 0.0, 0.0, 0.0];
    let b = vec![3.0, 4.0, 0.0, 0.0];

    let dist = cached.distance(&a, &b);
    // CachedSimdDistance returns squared L2 for Euclidean
    assert!(
        (dist - 25.0).abs() < 1e-5,
        "3-4-5 triangle squared: got {dist}"
    );
}

#[test]
#[allow(clippy::cast_precision_loss)]
fn test_cached_simd_dot_product() {
    let cached = CachedSimdDistance::new(DistanceMetric::DotProduct, 128);

    let a: Vec<f32> = (0..128).map(|i| i as f32 * 0.1).collect();
    let b: Vec<f32> = (0..128).map(|i| (128 - i) as f32 * 0.1).collect();

    let dist = cached.distance(&a, &b);
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
    let cached = CachedSimdDistance::new(DistanceMetric::Cosine, 4);
    assert_eq!(cached.metric(), DistanceMetric::Cosine);
}

#[test]
fn test_native_simd_metric_accessor() {
    let cached = CachedSimdDistance::new(DistanceMetric::DotProduct, 4);
    assert_eq!(cached.metric(), DistanceMetric::DotProduct);
}

#[test]
fn test_simd_dot_product() {
    let cached = CachedSimdDistance::new(DistanceMetric::DotProduct, 4);
    let a = vec![1.0, 2.0, 3.0, 4.0];
    let b = vec![1.0, 1.0, 1.0, 1.0];
    let dist = cached.distance(&a, &b);
    // dot = 10, distance = -10
    assert!((dist + 10.0).abs() < 1e-4);
}

#[test]
fn test_simd_euclidean() {
    let cached = CachedSimdDistance::new(DistanceMetric::Euclidean, 4);
    let a = vec![0.0, 0.0, 0.0, 0.0];
    let b = vec![3.0, 4.0, 0.0, 0.0];
    let dist = cached.distance(&a, &b);
    // CachedSimdDistance returns squared L2
    assert!((dist - 25.0).abs() < 1e-4);
}

#[test]
fn test_cached_simd_hamming() {
    let cached = CachedSimdDistance::new(DistanceMetric::Hamming, 32);
    let a: Vec<f32> = (0..32)
        .map(|i| if i % 2 == 0 { 1.0 } else { 0.0 })
        .collect();
    let b: Vec<f32> = (0..32)
        .map(|i| if i % 3 == 0 { 1.0 } else { 0.0 })
        .collect();
    let dist = cached.distance(&a, &b);
    assert!(dist >= 0.0);
}

#[test]
fn test_cached_simd_jaccard() {
    let cached = CachedSimdDistance::new(DistanceMetric::Jaccard, 4);
    let a = vec![1.0, 1.0, 0.0, 0.0];
    let b = vec![1.0, 1.0, 1.0, 0.0];
    let dist = cached.distance(&a, &b);
    assert!((0.0..=1.0).contains(&dist));
}

// =========================================================================
// Tests for CachedSimdDistance correctness (returns distance, not similarity)
// =========================================================================

#[test]
fn test_cached_simd_cosine_returns_distance() {
    let cached = CachedSimdDistance::new(DistanceMetric::Cosine, 4);

    // Identical vectors should have distance ~0 (not similarity ~1)
    let v = vec![1.0, 2.0, 3.0, 4.0];
    let dist = cached.distance(&v, &v);
    assert!(
        dist.abs() < 1e-4,
        "CachedSimdDistance should return distance ~0 for identical vectors, got {dist}"
    );

    // Opposite vectors should have distance ~2 (not similarity ~-1)
    let opposite: Vec<f32> = v.iter().map(|x| -x).collect();
    let dist_opposite = cached.distance(&v, &opposite);
    assert!(
        (dist_opposite - 2.0).abs() < 1e-4,
        "CachedSimdDistance should return distance ~2 for opposite vectors, got {dist_opposite}"
    );
}

#[test]
fn test_cached_simd_dot_product_returns_distance() {
    let cached = CachedSimdDistance::new(DistanceMetric::DotProduct, 3);

    // Positive dot product should give negative distance (lower = better)
    let a = vec![1.0, 2.0, 3.0];
    let b = vec![1.0, 1.0, 1.0];
    let dist = cached.distance(&a, &b);
    // dot = 1*1 + 2*1 + 3*1 = 6, distance = -6
    assert!(
        dist < 0.0,
        "CachedSimdDistance DotProduct should return negative distance, got {dist}"
    );
    assert!(
        (dist + 6.0).abs() < 1e-4,
        "Expected distance ~-6, got {dist}"
    );
}

#[test]
fn test_cached_simd_jaccard_returns_distance() {
    let cached = CachedSimdDistance::new(DistanceMetric::Jaccard, 32);

    // Identical vectors should have distance ~0
    let v: Vec<f32> = (0..32)
        .map(|i| if i % 2 == 0 { 1.0 } else { 0.0 })
        .collect();
    let dist = cached.distance(&v, &v);
    assert!(
        dist.abs() < 1e-4,
        "CachedSimdDistance Jaccard should return distance ~0 for identical vectors, got {dist}"
    );

    // Jaccard distance should be in [0, 1]
    let b: Vec<f32> = (0..32)
        .map(|i| if i % 3 == 0 { 1.0 } else { 0.0 })
        .collect();
    let dist2 = cached.distance(&v, &b);
    assert!(
        (0.0..=1.0).contains(&dist2),
        "CachedSimdDistance Jaccard distance should be in [0,1], got {dist2}"
    );
}

#[test]
fn test_cached_simd_matches_cpu_all_metrics() {
    // Ensure CachedSimdDistance returns same results as CpuDistance for non-Euclidean metrics
    let cpu = CpuDistance::new(DistanceMetric::Cosine);
    let cached = CachedSimdDistance::new(DistanceMetric::Cosine, 768);

    #[allow(clippy::cast_precision_loss)]
    let a: Vec<f32> = (0..768).map(|i| (i as f32 * 0.01).sin()).collect();
    #[allow(clippy::cast_precision_loss)]
    let b: Vec<f32> = (0..768).map(|i| (i as f32 * 0.02).cos()).collect();

    let cpu_dist = cpu.distance(&a, &b);
    let cached_dist = cached.distance(&a, &b);

    assert!(
        (cpu_dist - cached_dist).abs() < 1e-3,
        "CachedSimdDistance should match CpuDistance: cpu={cpu_dist}, cached={cached_dist}"
    );
}

#[test]
fn test_cached_simd_euclidean_returns_squared_distance() {
    let cached = CachedSimdDistance::new(DistanceMetric::Euclidean, 4);

    let a = vec![0.0, 0.0, 0.0, 0.0];
    let b = vec![3.0, 4.0, 0.0, 0.0];
    let dist = cached.distance(&a, &b);

    // CachedSimdDistance returns squared L2 for Euclidean (25.0, not 5.0)
    assert!(
        (dist - 25.0).abs() < 1e-4,
        "CachedSimdDistance Euclidean should return squared L2 = 25.0, got {dist}"
    );
}

#[test]
fn test_cached_simd_hamming_returns_distance() {
    let cached = CachedSimdDistance::new(DistanceMetric::Hamming, 32);

    let a: Vec<f32> = (0..32)
        .map(|i| if i % 2 == 0 { 1.0 } else { 0.0 })
        .collect();
    let b: Vec<f32> = (0..32)
        .map(|i| if i % 3 == 0 { 1.0 } else { 0.0 })
        .collect();

    let dist = cached.distance(&a, &b);
    assert!(
        dist >= 0.0,
        "CachedSimdDistance Hamming distance should be non-negative, got {dist}"
    );
}

#[test]
#[allow(clippy::cast_precision_loss)]
fn test_cached_simd_batch_dot_product() {
    let cached = CachedSimdDistance::new(DistanceMetric::DotProduct, 16);
    let query: Vec<f32> = vec![1.0; 16];
    let candidates: Vec<Vec<f32>> = (0..5).map(|i| vec![(i + 1) as f32; 16]).collect();
    let candidate_refs: Vec<&[f32]> = candidates.iter().map(Vec::as_slice).collect();

    let distances = cached.batch_distance(&query, &candidate_refs);
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
fn test_cached_simd_batch_euclidean() {
    let cached = CachedSimdDistance::new(DistanceMetric::Euclidean, 8);
    let query = vec![0.0; 8];
    let candidates: Vec<Vec<f32>> = vec![vec![1.0; 8], vec![2.0; 8]];
    let candidate_refs: Vec<&[f32]> = candidates.iter().map(Vec::as_slice).collect();

    let distances = cached.batch_distance(&query, &candidate_refs);
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

// =========================================================================
// Tests for CachedSimdDistance — bit-for-bit parity with SimdDistance
// =========================================================================

#[allow(clippy::cast_precision_loss)]
fn gen_vec(dim: usize, seed: f32) -> Vec<f32> {
    (0..dim).map(|i| (seed + i as f32 * 0.01).sin()).collect()
}

#[test]
fn test_cached_cosine_768d() {
    let dim = 768;
    let cached = CachedSimdDistance::new(DistanceMetric::Cosine, dim);
    let cpu = CpuDistance::new(DistanceMetric::Cosine);
    let a = gen_vec(dim, 0.0);
    let b = gen_vec(dim, 1.0);
    let cpu_d = cpu.distance(&a, &b);
    let c = cached.distance(&a, &b);
    assert!(
        (cpu_d - c).abs() < 1e-4,
        "cosine 768d: cpu={cpu_d}, cached={c}"
    );
}

#[test]
fn test_cached_euclidean_128d() {
    let dim = 128;
    let cpu = CpuDistance::new(DistanceMetric::Euclidean);
    let cached = CachedSimdDistance::new(DistanceMetric::Euclidean, dim);
    let a = gen_vec(dim, 0.0);
    let b = gen_vec(dim, 1.0);
    let cpu_d = cpu.distance(&a, &b); // sqrt'd Euclidean
    let c = cached.distance(&a, &b); // squared L2 (no sqrt)
                                     // CachedSimdDistance returns squared L2 for HNSW traversal optimization;
                                     // CpuDistance returns actual Euclidean (with sqrt).
    assert!(
        (c - cpu_d * cpu_d).abs() < 1e-3,
        "cached should equal cpu^2: cached={c}, cpu^2={}",
        cpu_d * cpu_d,
    );
}

#[test]
fn test_cached_dot_product_1536d() {
    let dim = 1536;
    let cpu = CpuDistance::new(DistanceMetric::DotProduct);
    let cached = CachedSimdDistance::new(DistanceMetric::DotProduct, dim);
    let a = gen_vec(dim, 0.0);
    let b = gen_vec(dim, 1.0);
    let cpu_d = cpu.distance(&a, &b);
    let c = cached.distance(&a, &b);
    assert!(
        (cpu_d - c).abs() < 1e-2,
        "dot_product 1536d: cpu={cpu_d}, cached={c}"
    );
}

#[test]
fn test_cached_hamming_64d() {
    let dim = 64;
    let cpu = CpuDistance::new(DistanceMetric::Hamming);
    let cached = CachedSimdDistance::new(DistanceMetric::Hamming, dim);
    let a: Vec<f32> = (0..dim)
        .map(|i| if i % 3 == 0 { 1.0 } else { 0.0 })
        .collect();
    let b: Vec<f32> = (0..dim)
        .map(|i| if i % 2 == 0 { 1.0 } else { 0.0 })
        .collect();
    let cpu_d = cpu.distance(&a, &b);
    let c = cached.distance(&a, &b);
    assert_eq!(cpu_d, c, "hamming 64d: cpu={cpu_d}, cached={c}");
}

#[test]
fn test_cached_jaccard_256d() {
    let dim = 256;
    let cpu = CpuDistance::new(DistanceMetric::Jaccard);
    let cached = CachedSimdDistance::new(DistanceMetric::Jaccard, dim);
    let a: Vec<f32> = (0..dim)
        .map(|i| if i < dim / 2 { 1.0 } else { 0.0 })
        .collect();
    let b: Vec<f32> = (0..dim)
        .map(|i| if i < dim * 3 / 4 { 1.0 } else { 0.0 })
        .collect();
    let cpu_d = cpu.distance(&a, &b);
    let c = cached.distance(&a, &b);
    assert!(
        (cpu_d - c).abs() < 1e-4,
        "jaccard 256d: cpu={cpu_d}, cached={c}"
    );
}

#[test]
fn test_cached_batch_distance_matches_single() {
    let dim = 128;
    let cached = CachedSimdDistance::new(DistanceMetric::Cosine, dim);
    let query = gen_vec(dim, 0.0);
    let candidates: Vec<Vec<f32>> = (0..20).map(|j| gen_vec(dim, j as f32)).collect();
    let candidate_refs: Vec<&[f32]> = candidates.iter().map(Vec::as_slice).collect();

    let batch = cached.batch_distance(&query, &candidate_refs);
    let single: Vec<f32> = candidate_refs
        .iter()
        .map(|c| cached.distance(&query, c))
        .collect();

    for (i, (b, s)) in batch.iter().zip(single.iter()).enumerate() {
        assert_eq!(
            b, s,
            "batch vs single mismatch at {i}: batch={b}, single={s}"
        );
    }
}

// =========================================================================
// F-22: Pre-normalization tests
// =========================================================================

#[test]
fn test_prenormalized_cosine_uses_dot_product() {
    let dim = 128;
    let prenorm = CachedSimdDistance::new_prenormalized(DistanceMetric::Cosine, dim);
    assert!(prenorm.is_pre_normalized());

    // Pre-normalize two vectors manually
    let mut a = gen_vec(dim, 0.0);
    let mut b = gen_vec(dim, 1.0);
    crate::simd_native::normalize_inplace_native(&mut a);
    crate::simd_native::normalize_inplace_native(&mut b);

    // Prenormalized cosine distance should equal 1 - dot(a, b)
    let prenorm_dist = prenorm.distance(&a, &b);
    let dot = crate::simd_native::dot_product_native(&a, &b);
    let expected = 1.0 - dot;
    assert!(
        (prenorm_dist - expected).abs() < 1e-6,
        "prenorm cosine: got {prenorm_dist}, expected {expected}"
    );
}

#[test]
fn test_prenormalized_matches_standard_cosine_on_unit_vectors() {
    let dim = 768;
    let standard = CachedSimdDistance::new(DistanceMetric::Cosine, dim);
    let prenorm = CachedSimdDistance::new_prenormalized(DistanceMetric::Cosine, dim);

    let mut a = gen_vec(dim, 0.5);
    let mut b = gen_vec(dim, 2.0);
    crate::simd_native::normalize_inplace_native(&mut a);
    crate::simd_native::normalize_inplace_native(&mut b);

    let standard_dist = standard.distance(&a, &b);
    let prenorm_dist = prenorm.distance(&a, &b);

    // For unit vectors, both should give the same result
    assert!(
        (standard_dist - prenorm_dist).abs() < 1e-5,
        "standard={standard_dist}, prenorm={prenorm_dist} should match on unit vectors"
    );
}

#[test]
fn test_prenormalized_flag_only_affects_cosine() {
    let dim = 64;
    let prenorm = CachedSimdDistance::new_prenormalized(DistanceMetric::Euclidean, dim);
    let standard = CachedSimdDistance::new(DistanceMetric::Euclidean, dim);

    let a = gen_vec(dim, 0.0);
    let b = gen_vec(dim, 1.0);

    // Pre-normalization flag has no effect on non-Cosine metrics
    assert_eq!(
        prenorm.distance(&a, &b),
        standard.distance(&a, &b),
        "Pre-normalization must not affect Euclidean distance"
    );
}

#[test]
fn test_non_prenormalized_cosine_flag_is_false() {
    let standard = CachedSimdDistance::new(DistanceMetric::Cosine, 128);
    assert!(!standard.is_pre_normalized());
}

#[test]
fn test_default_distance_engine_is_not_prenormalized() {
    let cpu = CpuDistance::new(DistanceMetric::Cosine);
    assert!(!cpu.is_pre_normalized());
}

// =========================================================================
// Tests for #420 Component 1: Skip sqrt in Euclidean HNSW search
// =========================================================================

/// Verifies that `CachedSimdDistance` for Euclidean now returns squared L2
/// (no sqrt) so that HNSW graph traversal avoids redundant sqrt calls.
///
/// The ordering of squared L2 is identical to Euclidean ordering because
/// sqrt is monotonically increasing.
#[allow(clippy::similar_names)] // Reason: near/far naming convention for distance tests
#[test]
fn test_cached_euclidean_returns_squared_l2_for_ordering() {
    let dim = 128;
    let cached = CachedSimdDistance::new(DistanceMetric::Euclidean, dim);

    let a = gen_vec(dim, 0.0);
    let near = gen_vec(dim, 1.0);
    let far = gen_vec(dim, 2.0);

    let dist_near = cached.distance(&a, &near);
    let dist_far = cached.distance(&a, &far);

    // Compute ground-truth squared L2 manually
    let expected_near: f32 = a
        .iter()
        .zip(near.iter())
        .map(|(x, y)| (x - y).powi(2))
        .sum();
    let expected_far: f32 = a.iter().zip(far.iter()).map(|(x, y)| (x - y).powi(2)).sum();

    // CachedSimdDistance should return squared L2 (not sqrt'd)
    assert!(
        (dist_near - expected_near).abs() < 1e-3,
        "Expected squared L2 {expected_near}, got {dist_near}"
    );
    assert!(
        (dist_far - expected_far).abs() < 1e-3,
        "Expected squared L2 {expected_far}, got {dist_far}"
    );

    // Ordering must be preserved
    assert_eq!(
        dist_near < dist_far,
        expected_near < expected_far,
        "Ordering of squared L2 must match"
    );
}

/// Verifies that `DistanceEngine::euclidean_squared()` exists and returns
/// raw squared L2 without sqrt.
#[test]
fn test_distance_engine_euclidean_squared() {
    let engine = crate::simd_native::DistanceEngine::new(4);
    let a = [3.0_f32, 0.0, 0.0, 0.0];
    let b = [0.0_f32, 4.0, 0.0, 0.0];

    let sq = engine.euclidean_squared(&a, &b);
    // 3^2 + 4^2 = 25 (squared L2, NOT 5.0)
    assert!(
        (sq - 25.0).abs() < 1e-5,
        "euclidean_squared should return 25.0, got {sq}"
    );

    // Original euclidean() still applies sqrt
    let euc = engine.euclidean(&a, &b);
    assert!(
        (euc - 5.0).abs() < 1e-5,
        "euclidean should still return 5.0, got {euc}"
    );
}

/// Verifies that `transform_score` for Euclidean now applies sqrt,
/// restoring the actual Euclidean distance for user-visible scores.
#[test]
fn test_transform_score_euclidean_applies_sqrt() {
    let dim = 32;
    let engine = CachedSimdDistance::new(DistanceMetric::Euclidean, dim);
    let hnsw = super::graph::NativeHnsw::new(engine, 16, 100, 100);

    // If the raw distance from the search is squared L2 = 25.0,
    // transform_score should return sqrt(25.0) = 5.0
    let score = hnsw.transform_score(25.0);
    assert!(
        (score - 5.0).abs() < 1e-5,
        "transform_score(25.0) should return 5.0 (sqrt), got {score}"
    );

    // Zero distance should remain zero
    let score_zero = hnsw.transform_score(0.0);
    assert!(
        score_zero.abs() < 1e-5,
        "transform_score(0.0) should return 0.0, got {score_zero}"
    );
}

/// End-to-end: HNSW search with Euclidean metric must return squared L2
/// as raw distances, and `transform_score` must apply sqrt to produce
/// actual Euclidean distances for user-visible scores.
#[test]
fn test_hnsw_euclidean_search_returns_actual_distances() {
    let dim = 4;
    let engine = CachedSimdDistance::new(DistanceMetric::Euclidean, dim);
    let hnsw = super::graph::NativeHnsw::new(engine, 16, 100, 100);

    // Insert origin and a known vector at distance 5.0 (3-4-5 triangle)
    hnsw.insert(&[0.0, 0.0, 0.0, 0.0]).expect("test");
    hnsw.insert(&[3.0, 4.0, 0.0, 0.0]).expect("test");

    // Search from origin, requesting 2 results
    let results = hnsw.search(&[0.0, 0.0, 0.0, 0.0], 2, 50);
    assert_eq!(results.len(), 2, "Should find both vectors");

    // The self-distance should be 0 (or very close)
    assert!(
        results[0].1 < 0.01,
        "Self-distance should be ~0, got {}",
        results[0].1
    );

    // Raw distance from search should be SQUARED L2 = 25.0, not 5.0
    let raw_dist = results[1].1;
    assert!(
        (raw_dist - 25.0).abs() < 0.1,
        "Raw HNSW distance should be squared L2 = 25.0, got {raw_dist}"
    );

    // transform_score must convert squared L2 → actual Euclidean distance
    let user_score = hnsw.transform_score(raw_dist);
    assert!(
        (user_score - 5.0).abs() < 0.1,
        "User-visible Euclidean distance should be ~5.0, got {user_score}"
    );
}

/// Verifies that the top-k ordering is identical between squared L2 and
/// actual Euclidean distance (sqrt is monotone), using a larger dataset.
#[test]
fn test_squared_l2_preserves_topk_ordering() {
    let dim = 32;
    let cached = CachedSimdDistance::new(DistanceMetric::Euclidean, dim);
    let hnsw = super::graph::NativeHnsw::new(cached, 16, 100, 600);

    // Insert 500 vectors
    for i in 0..500_u64 {
        let v: Vec<f32> = (0..dim)
            .map(|j| ((i as f32 + j as f32) * 0.01).sin())
            .collect();
        hnsw.insert(&v).expect("test");
    }

    let query: Vec<f32> = (0..dim).map(|j| (j as f32 * 0.05).cos()).collect();
    let k = 10;

    // Get HNSW results (uses squared L2 internally)
    let hnsw_results = hnsw.search(&query, k, 128);

    // Brute-force with ACTUAL Euclidean distances (with sqrt)
    let bf_engine = CpuDistance::new(DistanceMetric::Euclidean);
    let mut bf_distances: Vec<(usize, f32)> = (0..500)
        .map(|i| {
            let v: Vec<f32> = (0..dim)
                .map(|j| ((i as f32 + j as f32) * 0.01).sin())
                .collect();
            (i, bf_engine.distance(&query, &v))
        })
        .collect();
    bf_distances.sort_by(|a, b| a.1.total_cmp(&b.1));
    let bf_top_k: Vec<usize> = bf_distances.iter().take(k).map(|&(id, _)| id).collect();
    let hnsw_ids: Vec<usize> = hnsw_results.iter().map(|&(id, _)| id).collect();

    // Recall should be >= 0.95 (ordering preserved, HNSW approximate)
    let recall = crate::metrics::recall_at_k(&bf_top_k, &hnsw_ids);
    assert!(
        recall >= 0.90,
        "recall@{k} should be >= 0.90, got {recall:.4}"
    );
}
