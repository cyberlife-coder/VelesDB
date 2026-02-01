//! Tests for EPIC-073 SIMD Pipeline Optimizations.
//!
//! US-002: Jaccard SIMD optimization
//! US-004: Cache alignment (verified in storage tests)
//! US-005: Quantization auto-enable

use crate::config::QuantizationConfig;
use crate::simd_native::jaccard_similarity_native;

// =============================================================================
// US-002: Jaccard SIMD Tests
// =============================================================================

#[test]
fn test_jaccard_simd_identical_sets() {
    let a: Vec<f32> = vec![1.0, 0.0, 1.0, 1.0, 0.0, 1.0, 0.0, 0.0];
    let b: Vec<f32> = vec![1.0, 0.0, 1.0, 1.0, 0.0, 1.0, 0.0, 0.0];
    let result = jaccard_similarity_native(&a, &b);
    assert!(
        (result - 1.0).abs() < 1e-5,
        "Identical sets should have Jaccard = 1.0"
    );
}

#[test]
fn test_jaccard_simd_disjoint_sets() {
    let a: Vec<f32> = vec![1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0];
    let b: Vec<f32> = vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0];
    let result = jaccard_similarity_native(&a, &b);
    assert!(
        (result - 0.0).abs() < 1e-5,
        "Disjoint sets should have Jaccard = 0.0"
    );
}

#[test]
fn test_jaccard_simd_half_overlap() {
    let a: Vec<f32> = vec![1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
    let b: Vec<f32> = vec![1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0];
    let result = jaccard_similarity_native(&a, &b);
    // Intersection = 1, Union = 3
    assert!((result - 1.0 / 3.0).abs() < 1e-5, "Expected Jaccard = 1/3");
}

#[test]
fn test_jaccard_simd_empty_sets() {
    let a: Vec<f32> = vec![0.0; 16];
    let b: Vec<f32> = vec![0.0; 16];
    let result = jaccard_similarity_native(&a, &b);
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
    let result = jaccard_similarity_native(&a, &b);
    assert!(
        result > 0.0 && result < 1.0,
        "Jaccard should be between 0 and 1"
    );
}

// Binary Jaccard tests removed - function not migrated to simd_native (EPIC-075)

// =============================================================================
// US-003: Batch Similarity Tests
// Note: batch_dot_product and batch_similarity_top_k removed in EPIC-075
// These functions were specific to simd_explicit and not migrated.
// Batch operations are now handled via simd_native::batch_dot_product_native.
// =============================================================================

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
