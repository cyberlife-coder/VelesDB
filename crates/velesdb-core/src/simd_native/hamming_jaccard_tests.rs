#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::float_cmp
)]
//! Comprehensive tests for Hamming and Jaccard SIMD kernels.
//!
//! Validates that SIMD-dispatched implementations match scalar reference
//! implementations across all dimension thresholds, edge cases, batch
//! operations, and `DistanceEngine` cached dispatch.

use super::dispatch::{
    batch_hamming_native, batch_jaccard_native, hamming_distance_native, jaccard_similarity_native,
    DistanceEngine,
};
use super::scalar;

/// Tolerance for Jaccard floating-point comparison.
const JACCARD_EPS: f32 = 1e-4;

// =============================================================================
// Helpers
// =============================================================================

/// Deterministic binary-ish test vector: `a[i] = 1.0` when `i % modulus == 0`.
fn make_pattern_vector(dim: usize, modulus: usize) -> Vec<f32> {
    (0..dim)
        .map(|i| if i % modulus == 0 { 1.0 } else { 0.0 })
        .collect()
}

/// Assert Hamming native matches scalar for a single dimension.
fn assert_hamming_matches_scalar(dim: usize) {
    let a = make_pattern_vector(dim, 3);
    let b = make_pattern_vector(dim, 2);
    let native = hamming_distance_native(&a, &b);
    let reference = scalar::hamming_scalar(&a, &b);
    assert_eq!(
        native, reference,
        "hamming mismatch at dim={dim}: native={native}, scalar={reference}"
    );
}

/// Assert Jaccard native matches scalar for a single dimension.
fn assert_jaccard_matches_scalar(dim: usize) {
    let a = make_pattern_vector(dim, 3);
    let b = make_pattern_vector(dim, 2);
    let native = jaccard_similarity_native(&a, &b);
    let reference = scalar::jaccard_scalar(&a, &b);
    assert!(
        (native - reference).abs() < JACCARD_EPS,
        "jaccard mismatch at dim={dim}: native={native}, scalar={reference}"
    );
}

// =============================================================================
// 1. Scalar-vs-native consistency across all dimension thresholds
// =============================================================================

/// Dimensions that hit every SIMD dispatch tier boundary:
/// scalar (<4/8), NEON (>=4), AVX2 (>=8), AVX-512 1-acc (>=16),
/// AVX-512 4-acc (>=512), AVX-512 8-acc (>=1024), plus common embedding dims.
const THRESHOLD_DIMS: &[usize] = &[
    1, 3, 4, 7, 8, 15, 16, 31, 32, 63, 64, 127, 128, 255, 256, 511, 512, 768, 1023, 1024, 1025,
    1536, 2048,
];

#[test]
fn test_hamming_native_matches_scalar_all_thresholds() {
    for &dim in THRESHOLD_DIMS {
        assert_hamming_matches_scalar(dim);
    }
}

#[test]
fn test_jaccard_native_matches_scalar_all_thresholds() {
    for &dim in THRESHOLD_DIMS {
        assert_jaccard_matches_scalar(dim);
    }
}

// =============================================================================
// 2. Edge cases
// =============================================================================

#[test]
fn test_hamming_identical_vectors() {
    for dim in [1, 8, 16, 64, 128, 512] {
        let a: Vec<f32> = (0..dim)
            .map(|i| if i % 2 == 0 { 1.0 } else { 0.0 })
            .collect();
        let result = hamming_distance_native(&a, &a);
        assert_eq!(
            result, 0.0,
            "identical vectors at dim={dim} should have hamming=0"
        );
    }
}

#[test]
fn test_jaccard_identical_vectors() {
    for dim in [1, 8, 16, 64, 128, 512] {
        let a: Vec<f32> = (0..dim)
            .map(|i| if i % 2 == 0 { 1.0 } else { 0.0 })
            .collect();
        let result = jaccard_similarity_native(&a, &a);
        assert!(
            (result - 1.0).abs() < JACCARD_EPS,
            "identical vectors at dim={dim} should have jaccard=1.0, got {result}"
        );
    }
}

#[test]
fn test_hamming_all_different() {
    for dim in [1, 8, 16, 64, 128, 512] {
        let a = vec![1.0_f32; dim];
        let b = vec![0.0_f32; dim];
        let result = hamming_distance_native(&a, &b);
        assert_eq!(
            result, dim as f32,
            "all-different at dim={dim}: expected {dim}, got {result}"
        );
    }
}

#[test]
fn test_jaccard_all_different() {
    // a = all 1.0, b = all 0.0 => intersection = sum(min) = 0, union = sum(max) = dim
    for dim in [1, 8, 16, 64, 128, 512] {
        let a = vec![1.0_f32; dim];
        let b = vec![0.0_f32; dim];
        let result = jaccard_similarity_native(&a, &b);
        assert!(
            result.abs() < JACCARD_EPS,
            "all-different at dim={dim}: jaccard should be 0.0, got {result}"
        );
    }
}

#[test]
fn test_hamming_all_zero_vectors() {
    for dim in [1, 8, 16, 64, 128, 512] {
        let a = vec![0.0_f32; dim];
        let b = vec![0.0_f32; dim];
        let result = hamming_distance_native(&a, &b);
        assert_eq!(result, 0.0, "all-zero at dim={dim}: hamming should be 0.0");
    }
}

#[test]
fn test_jaccard_all_zero_vectors() {
    // Both vectors all-zero => union=0 => div-by-zero guard should return 1.0
    for dim in [1, 8, 16, 64, 128, 512] {
        let a = vec![0.0_f32; dim];
        let b = vec![0.0_f32; dim];
        let result = jaccard_similarity_native(&a, &b);
        assert!(
            (result - 1.0).abs() < JACCARD_EPS,
            "all-zero at dim={dim}: jaccard should be 1.0 (div-by-zero guard), got {result}"
        );
    }
}

/// Odd-length dimensions exercise SIMD remainder/tail handling.
#[test]
fn test_hamming_odd_lengths() {
    for dim in [1, 3, 5, 7, 9, 13, 15, 17, 33] {
        assert_hamming_matches_scalar(dim);
    }
}

#[test]
fn test_jaccard_odd_lengths() {
    for dim in [1, 3, 5, 7, 9, 13, 15, 17, 33] {
        assert_jaccard_matches_scalar(dim);
    }
}

#[test]
fn test_hamming_single_element() {
    // Both above threshold => same => hamming = 0
    assert_eq!(hamming_distance_native(&[1.0], &[1.0]), 0.0);
    // One above, one below => different => hamming = 1
    assert_eq!(hamming_distance_native(&[1.0], &[0.0]), 1.0);
    // Both below threshold => same => hamming = 0
    assert_eq!(hamming_distance_native(&[0.0], &[0.0]), 0.0);
}

#[test]
fn test_jaccard_single_element() {
    // min(1,1)/max(1,1) = 1.0
    let j_same = jaccard_similarity_native(&[1.0], &[1.0]);
    assert!((j_same - 1.0).abs() < JACCARD_EPS);
    // min(1,0)/max(1,0) = 0/1 = 0.0
    let j_diff = jaccard_similarity_native(&[1.0], &[0.0]);
    assert!(j_diff.abs() < JACCARD_EPS);
    // min(0,0)/max(0,0) = 0/0 => guard => 1.0
    let j_zero = jaccard_similarity_native(&[0.0], &[0.0]);
    assert!((j_zero - 1.0).abs() < JACCARD_EPS);
}

// =============================================================================
// 3. DistanceEngine consistency at large dims
// =============================================================================

const ENGINE_DIMS: &[usize] = &[512, 768, 1024, 1536];

#[test]
fn test_engine_hamming_matches_native_large_dims() {
    for &dim in ENGINE_DIMS {
        let engine = DistanceEngine::new(dim);
        let a = make_pattern_vector(dim, 3);
        let b = make_pattern_vector(dim, 2);
        let cached = engine.hamming(&a, &b);
        let native = hamming_distance_native(&a, &b);
        assert_eq!(
            cached, native,
            "engine hamming mismatch at dim={dim}: cached={cached}, native={native}"
        );
    }
}

#[test]
fn test_engine_jaccard_matches_native_large_dims() {
    for &dim in ENGINE_DIMS {
        let engine = DistanceEngine::new(dim);
        let a = make_pattern_vector(dim, 3);
        let b = make_pattern_vector(dim, 2);
        let cached = engine.jaccard(&a, &b);
        let native = jaccard_similarity_native(&a, &b);
        assert!(
            (cached - native).abs() < JACCARD_EPS,
            "engine jaccard mismatch at dim={dim}: cached={cached}, native={native}"
        );
    }
}

// =============================================================================
// 4. Batch operations
// =============================================================================

/// Build 100 candidate vectors at the given dimension, each with a unique seed.
fn build_candidates(dim: usize, count: usize) -> Vec<Vec<f32>> {
    (0..count)
        .map(|seed| {
            (0..dim)
                .map(|i| if (i + seed) % 3 == 0 { 1.0 } else { 0.0 })
                .collect()
        })
        .collect()
}

/// Assert batch hamming matches individual calls for one set of candidates.
fn assert_batch_hamming_matches_individual(dim: usize, count: usize) {
    let owned = build_candidates(dim, count);
    let candidates: Vec<&[f32]> = owned.iter().map(Vec::as_slice).collect();
    let query = make_pattern_vector(dim, 2);
    let batch = batch_hamming_native(&candidates, &query);

    assert_eq!(batch.len(), count);
    for (i, &batch_val) in batch.iter().enumerate() {
        let individual = hamming_distance_native(candidates[i], &query);
        assert_eq!(
            batch_val, individual,
            "batch hamming mismatch at index {i}, dim={dim}"
        );
    }
}

/// Assert batch jaccard matches individual calls for one set of candidates.
fn assert_batch_jaccard_matches_individual(dim: usize, count: usize) {
    let owned = build_candidates(dim, count);
    let candidates: Vec<&[f32]> = owned.iter().map(Vec::as_slice).collect();
    let query = make_pattern_vector(dim, 2);
    let batch = batch_jaccard_native(&candidates, &query);

    assert_eq!(batch.len(), count);
    for (i, &batch_val) in batch.iter().enumerate() {
        let individual = jaccard_similarity_native(candidates[i], &query);
        assert!(
            (batch_val - individual).abs() < JACCARD_EPS,
            "batch jaccard mismatch at index {i}, dim={dim}: batch={batch_val}, individual={individual}"
        );
    }
}

#[test]
fn test_batch_hamming_matches_individual_calls() {
    assert_batch_hamming_matches_individual(384, 100);
}

#[test]
fn test_batch_jaccard_matches_individual_calls() {
    assert_batch_jaccard_matches_individual(384, 100);
}

#[test]
fn test_batch_hamming_empty_candidates() {
    let candidates: Vec<&[f32]> = vec![];
    let query = vec![1.0_f32; 16];
    let result = batch_hamming_native(&candidates, &query);
    assert!(result.is_empty());
}

#[test]
fn test_batch_jaccard_empty_candidates() {
    let candidates: Vec<&[f32]> = vec![];
    let query = vec![1.0_f32; 16];
    let result = batch_jaccard_native(&candidates, &query);
    assert!(result.is_empty());
}

#[test]
fn test_batch_hamming_single_candidate() {
    let candidate = vec![1.0_f32; 64];
    let query = vec![0.0_f32; 64];
    let candidates: Vec<&[f32]> = vec![&candidate];
    let batch = batch_hamming_native(&candidates, &query);
    assert_eq!(batch.len(), 1);
    assert_eq!(batch[0], hamming_distance_native(&candidate, &query));
}

#[test]
fn test_batch_jaccard_single_candidate() {
    let candidate = vec![1.0_f32; 64];
    let query = vec![0.0_f32; 64];
    let candidates: Vec<&[f32]> = vec![&candidate];
    let batch = batch_jaccard_native(&candidates, &query);
    assert_eq!(batch.len(), 1);
    let expected = jaccard_similarity_native(&candidate, &query);
    assert!((batch[0] - expected).abs() < JACCARD_EPS);
}

// =============================================================================
// 5. Hamming count precision at large dims
// =============================================================================

#[test]
fn test_hamming_exact_count_large_dim() {
    let dim = 10_000;
    // a[i] = 1.0 when i%3==0, else 0.0 (threshold > 0.5 => "set")
    // b[i] = 1.0 when i%2==0, else 0.0
    // Hamming counts positions where binary(a) != binary(b).
    let a = make_pattern_vector(dim, 3);
    let b = make_pattern_vector(dim, 2);

    // Count expected mismatches manually
    let expected: usize = (0..dim).filter(|&i| (i % 3 == 0) != (i % 2 == 0)).count();

    let result = hamming_distance_native(&a, &b);
    assert_eq!(
        result, expected as f32,
        "hamming at dim={dim}: expected exact count {expected}, got {result}"
    );
}

#[test]
fn test_hamming_exact_count_all_set() {
    // At dim=10000, all 1.0 vs all 0.0 => hamming = 10000 exactly
    let dim = 10_000;
    let a = vec![1.0_f32; dim];
    let b = vec![0.0_f32; dim];
    let result = hamming_distance_native(&a, &b);
    assert_eq!(
        result, dim as f32,
        "hamming at dim={dim}: expected {dim}, got {result}"
    );
}

#[test]
fn test_hamming_exact_count_none_set() {
    let dim = 10_000;
    let a = vec![0.0_f32; dim];
    let b = vec![0.0_f32; dim];
    let result = hamming_distance_native(&a, &b);
    assert_eq!(
        result, 0.0,
        "hamming at dim={dim}: expected 0, got {result}"
    );
}
