//! Tests for EPIC-073 SIMD Pipeline Optimizations.
//!
//! US-002: Jaccard SIMD optimization
//! US-003: Batch similarity search
//! US-004: Cache alignment (verified in storage tests)
//! US-005: Quantization auto-enable

use crate::config::QuantizationConfig;
use crate::simd_explicit::{
    batch_dot_product, batch_similarity_top_k, jaccard_similarity_binary, jaccard_similarity_simd,
};

// =============================================================================
// US-002: Jaccard SIMD Tests
// =============================================================================

#[test]
fn test_jaccard_simd_identical_sets() {
    let a: Vec<f32> = vec![1.0, 0.0, 1.0, 1.0, 0.0, 1.0, 0.0, 0.0];
    let b: Vec<f32> = vec![1.0, 0.0, 1.0, 1.0, 0.0, 1.0, 0.0, 0.0];
    let result = jaccard_similarity_simd(&a, &b);
    assert!(
        (result - 1.0).abs() < 1e-5,
        "Identical sets should have Jaccard = 1.0"
    );
}

#[test]
fn test_jaccard_simd_disjoint_sets() {
    let a: Vec<f32> = vec![1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0];
    let b: Vec<f32> = vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0];
    let result = jaccard_similarity_simd(&a, &b);
    assert!(
        (result - 0.0).abs() < 1e-5,
        "Disjoint sets should have Jaccard = 0.0"
    );
}

#[test]
fn test_jaccard_simd_half_overlap() {
    let a: Vec<f32> = vec![1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
    let b: Vec<f32> = vec![1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0];
    let result = jaccard_similarity_simd(&a, &b);
    // Intersection = 1, Union = 3
    assert!((result - 1.0 / 3.0).abs() < 1e-5, "Expected Jaccard = 1/3");
}

#[test]
fn test_jaccard_simd_empty_sets() {
    let a: Vec<f32> = vec![0.0; 16];
    let b: Vec<f32> = vec![0.0; 16];
    let result = jaccard_similarity_simd(&a, &b);
    assert!((result - 1.0).abs() < 1e-5, "Empty sets should return 1.0");
}

#[test]
fn test_jaccard_simd_large_vector() {
    // 768D vector - typical embedding size
    let a: Vec<f32> = (0..768)
        .map(|i| if i % 2 == 0 { 1.0 } else { 0.0 })
        .collect();
    let b: Vec<f32> = (0..768)
        .map(|i| if i % 3 == 0 { 1.0 } else { 0.0 })
        .collect();
    let result = jaccard_similarity_simd(&a, &b);
    assert!(
        result > 0.0 && result < 1.0,
        "Jaccard should be between 0 and 1"
    );
}

#[test]
fn test_jaccard_binary_identical() {
    let a: Vec<u64> = vec![0xFFFF_FFFF_FFFF_FFFF; 8];
    let b: Vec<u64> = vec![0xFFFF_FFFF_FFFF_FFFF; 8];
    let result = jaccard_similarity_binary(&a, &b);
    assert!(
        (result - 1.0).abs() < 1e-5,
        "Identical binary sets should have Jaccard = 1.0"
    );
}

#[test]
fn test_jaccard_binary_disjoint() {
    let a: Vec<u64> = vec![0xFFFF_FFFF_0000_0000; 8];
    let b: Vec<u64> = vec![0x0000_0000_FFFF_FFFF; 8];
    let result = jaccard_similarity_binary(&a, &b);
    assert!(
        (result - 0.0).abs() < 1e-5,
        "Disjoint binary sets should have Jaccard = 0.0"
    );
}

// =============================================================================
// US-003: Batch Similarity Tests
// =============================================================================

#[test]
fn test_batch_dot_product_basic() {
    let q1: Vec<f32> = vec![1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
    let q2: Vec<f32> = vec![0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
    let v1: Vec<f32> = vec![1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
    let v2: Vec<f32> = vec![0.5, 0.5, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];

    let queries: Vec<&[f32]> = vec![&q1, &q2];
    let vectors: Vec<&[f32]> = vec![&v1, &v2];

    let results = batch_dot_product(&queries, &vectors);

    assert_eq!(results.len(), 2);
    assert_eq!(results[0].len(), 2);
    assert!((results[0][0] - 1.0).abs() < 1e-5); // q1 · v1 = 1.0
    assert!((results[0][1] - 0.5).abs() < 1e-5); // q1 · v2 = 0.5
    assert!((results[1][0] - 1.0).abs() < 1e-5); // q2 · v1 = 1.0
    assert!((results[1][1] - 0.5).abs() < 1e-5); // q2 · v2 = 0.5
}

#[test]
fn test_batch_dot_product_empty() {
    let queries: Vec<&[f32]> = vec![];
    let vectors: Vec<&[f32]> = vec![];
    let results = batch_dot_product(&queries, &vectors);
    assert!(results.is_empty());
}

#[test]
fn test_batch_similarity_top_k() {
    let q1: Vec<f32> = vec![1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];

    let v1: Vec<f32> = vec![1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]; // dot = 1.0
    let v2: Vec<f32> = vec![0.5, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]; // dot = 0.5
    let v3: Vec<f32> = vec![0.8, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]; // dot = 0.8

    let queries: Vec<&[f32]> = vec![&q1];
    let vectors: Vec<(u64, &[f32])> = vec![(1, &v1[..]), (2, &v2[..]), (3, &v3[..])];

    let results = batch_similarity_top_k(&queries, &vectors, 2, true);

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].len(), 2);
    assert_eq!(results[0][0].0, 1); // Highest score
    assert_eq!(results[0][1].0, 3); // Second highest
}

#[test]
fn test_batch_similarity_top_k_distance() {
    let q1: Vec<f32> = vec![1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];

    let v1: Vec<f32> = vec![1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]; // dot = 1.0
    let v2: Vec<f32> = vec![0.5, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]; // dot = 0.5
    let v3: Vec<f32> = vec![0.8, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]; // dot = 0.8

    let queries: Vec<&[f32]> = vec![&q1];
    let vectors: Vec<(u64, &[f32])> = vec![(1, &v1[..]), (2, &v2[..]), (3, &v3[..])];

    // Lower is better (distance mode)
    let results = batch_similarity_top_k(&queries, &vectors, 2, false);

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].len(), 2);
    assert_eq!(results[0][0].0, 2); // Lowest score
    assert_eq!(results[0][1].0, 3); // Second lowest
}

// =============================================================================
// US-005: Auto-Quantization Config Tests
// =============================================================================

#[test]
fn test_quantization_config_default() {
    let config = QuantizationConfig::default();
    assert!(config.auto_quantization);
    assert_eq!(config.auto_quantization_threshold, 10_000);
}

#[test]
fn test_should_quantize_above_threshold() {
    let config = QuantizationConfig::default();
    assert!(config.should_quantize(15_000));
    assert!(config.should_quantize(10_000));
}

#[test]
fn test_should_quantize_below_threshold() {
    let config = QuantizationConfig::default();
    assert!(!config.should_quantize(9_999));
    assert!(!config.should_quantize(1_000));
}

#[test]
fn test_should_quantize_disabled() {
    let config = QuantizationConfig {
        auto_quantization: false,
        auto_quantization_threshold: 10_000,
        ..Default::default()
    };
    assert!(!config.should_quantize(50_000));
}

#[test]
fn test_should_quantize_custom_threshold() {
    let config = QuantizationConfig {
        auto_quantization: true,
        auto_quantization_threshold: 5_000,
        ..Default::default()
    };
    assert!(config.should_quantize(5_000));
    assert!(!config.should_quantize(4_999));
}
