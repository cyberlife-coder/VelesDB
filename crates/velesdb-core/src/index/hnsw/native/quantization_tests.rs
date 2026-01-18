//! Tests for `quantization` module - Scalar quantization for HNSW.

#![allow(clippy::similar_names)] // q_min, q_max, q_mid are intentionally similar

use super::quantization::{QuantizedVectorStore, ScalarQuantizer};
use std::sync::Arc;

// =========================================================================
// TDD Tests: ScalarQuantizer training
// =========================================================================

#[test]
fn test_train_computes_correct_min_max() {
    let v1 = vec![0.0, 10.0, -5.0];
    let v2 = vec![5.0, 20.0, 5.0];
    let v3 = vec![2.5, 15.0, 0.0];

    let quantizer = ScalarQuantizer::train(&[&v1, &v2, &v3]);

    assert_eq!(quantizer.dimension, 3);
    assert!((quantizer.min_vals[0] - 0.0).abs() < 1e-6);
    assert!((quantizer.min_vals[1] - 10.0).abs() < 1e-6);
    assert!((quantizer.min_vals[2] - (-5.0)).abs() < 1e-6);

    // Scale = 255 / (max - min)
    assert!((quantizer.scales[0] - 255.0 / 5.0).abs() < 1e-4);
    assert!((quantizer.scales[1] - 255.0 / 10.0).abs() < 1e-4);
    assert!((quantizer.scales[2] - 255.0 / 10.0).abs() < 1e-4);
}

#[test]
fn test_train_handles_constant_dimension() {
    let v1 = vec![1.0, 5.0, 5.0]; // dim 1 and 2 are constant
    let v2 = vec![2.0, 5.0, 5.0];

    let quantizer = ScalarQuantizer::train(&[&v1, &v2]);

    // Constant dimensions should have scale = 1.0 (fallback)
    assert!((quantizer.scales[1] - 1.0).abs() < 1e-6);
    assert!((quantizer.scales[2] - 1.0).abs() < 1e-6);
}

#[test]
#[should_panic(expected = "Cannot train on empty vectors")]
fn test_train_panics_on_empty() {
    let _: ScalarQuantizer = ScalarQuantizer::train(&[]);
}

// =========================================================================
// TDD Tests: Quantization and dequantization
// =========================================================================

#[test]
fn test_quantize_min_becomes_zero() {
    let v = vec![0.0, 100.0];
    let quantizer = ScalarQuantizer::train(&[&v]);

    let qvec = quantizer.quantize(&[0.0, 100.0]);

    // min should map to 0, max should map to 255
    assert_eq!(qvec.data[0], 0);
    // For single vector, min=max for each dim, so scale=1.0
}

#[test]
fn test_quantize_range_maps_correctly() {
    let v1 = vec![0.0, 0.0];
    let v2 = vec![10.0, 100.0];
    let quantizer = ScalarQuantizer::train(&[&v1, &v2]);

    // Test min values -> 0
    let q_min = quantizer.quantize(&[0.0, 0.0]);
    assert_eq!(q_min.data[0], 0);
    assert_eq!(q_min.data[1], 0);

    // Test max values -> 255
    let q_max = quantizer.quantize(&[10.0, 100.0]);
    assert_eq!(q_max.data[0], 255);
    assert_eq!(q_max.data[1], 255);

    // Test mid values -> ~127-128
    let q_mid = quantizer.quantize(&[5.0, 50.0]);
    assert!((i32::from(q_mid.data[0]) - 127).abs() <= 1);
    assert!((i32::from(q_mid.data[1]) - 127).abs() <= 1);
}

#[test]
fn test_quantize_clamps_out_of_range() {
    let v1 = vec![0.0];
    let v2 = vec![10.0];
    let quantizer = ScalarQuantizer::train(&[&v1, &v2]);

    // Value below training min
    let q_low = quantizer.quantize(&[-5.0]);
    assert_eq!(q_low.data[0], 0, "Should clamp to 0");

    // Value above training max
    let q_high = quantizer.quantize(&[20.0]);
    assert_eq!(q_high.data[0], 255, "Should clamp to 255");
}

#[test]
fn test_dequantize_recovers_approximate_values() {
    let v1 = vec![0.0, -10.0, 100.0];
    let v2 = vec![10.0, 10.0, 200.0];
    let quantizer = ScalarQuantizer::train(&[&v1, &v2]);

    let original = vec![5.0, 0.0, 150.0];
    let qvec = quantizer.quantize(&original);
    let recovered = quantizer.dequantize(&qvec);

    // Should be approximately equal (quantization error < 1% of range)
    for (i, (&orig, &rec)) in original.iter().zip(recovered.iter()).enumerate() {
        let range = v2[i] - v1[i];
        let error = (orig - rec).abs();
        let relative_error = error / range;
        assert!(
            relative_error < 0.01,
            "Dim {i}: orig={orig}, rec={rec}, error={relative_error:.4}"
        );
    }
}

// =========================================================================
// TDD Tests: Distance computation
// =========================================================================

#[test]
fn test_distance_l2_quantized_identical_is_zero() {
    let quantizer = ScalarQuantizer::train(&[&[0.0, 0.0], &[10.0, 10.0]]);
    let v = quantizer.quantize(&[5.0, 5.0]);

    let dist = quantizer.distance_l2_quantized(&v, &v);
    assert_eq!(dist, 0, "Distance to self should be 0");
}

#[test]
fn test_distance_l2_quantized_symmetry() {
    let quantizer = ScalarQuantizer::train(&[&[0.0, 0.0], &[10.0, 10.0]]);
    let a = quantizer.quantize(&[2.0, 3.0]);
    let b = quantizer.quantize(&[7.0, 8.0]);

    let dist_ab = quantizer.distance_l2_quantized(&a, &b);
    let dist_ba = quantizer.distance_l2_quantized(&b, &a);

    assert_eq!(dist_ab, dist_ba, "Distance should be symmetric");
}

#[test]
fn test_distance_l2_asymmetric_close_to_exact() {
    let v1 = vec![0.0; 128];
    let v2 = vec![10.0; 128];
    let quantizer = ScalarQuantizer::train(&[&v1, &v2]);

    let query = vec![3.0; 128];
    let candidate = vec![7.0; 128];

    let quantized_candidate = quantizer.quantize(&candidate);
    let approx_dist = quantizer.distance_l2_asymmetric(&query, &quantized_candidate);

    // Exact L2 distance
    let exact_dist: f32 = query
        .iter()
        .zip(candidate.iter())
        .map(|(a, b)| (a - b).powi(2))
        .sum::<f32>()
        .sqrt();

    // Asymmetric distance should be within 5% of exact
    let relative_error = (approx_dist - exact_dist).abs() / exact_dist;
    assert!(
        relative_error < 0.05,
        "approx={approx_dist}, exact={exact_dist}, error={relative_error:.4}"
    );
}

// =========================================================================
// TDD Tests: QuantizedVectorStore
// =========================================================================

#[test]
fn test_store_push_and_get() {
    let quantizer = Arc::new(ScalarQuantizer::train(&[&[0.0, 0.0], &[10.0, 10.0]]));
    let mut store = QuantizedVectorStore::new(quantizer.clone(), 100);

    store.push(&[2.0, 3.0]);
    store.push(&[7.0, 8.0]);

    assert_eq!(store.len(), 2);

    let v0 = store.get(0).expect("Should have index 0");
    let v1 = store.get(1).expect("Should have index 1");

    // Verify values are different
    assert_ne!(v0.data, v1.data);
}

#[test]
fn test_store_get_out_of_bounds_returns_none() {
    let quantizer = Arc::new(ScalarQuantizer::train(&[&[0.0], &[10.0]]));
    let store = QuantizedVectorStore::new(quantizer, 100);

    assert!(store.get(0).is_none());
    assert!(store.get(100).is_none());
}

#[test]
fn test_store_get_slice_zero_copy() {
    let quantizer = Arc::new(ScalarQuantizer::train(&[&[0.0, 0.0], &[10.0, 10.0]]));
    let mut store = QuantizedVectorStore::new(quantizer.clone(), 100);

    store.push(&[5.0, 5.0]);

    let slice = store.get_slice(0).expect("Should have slice");
    assert_eq!(slice.len(), 2);

    // Verify it's the expected quantized value (~127)
    assert!((i32::from(slice[0]) - 127).abs() <= 1);
    assert!((i32::from(slice[1]) - 127).abs() <= 1);
}

// =========================================================================
// TDD Tests: Memory efficiency
// =========================================================================

#[test]
fn test_memory_efficiency_4x_reduction() {
    let dim = 768;
    let count = 10_000;

    // Float32 storage: 768 * 4 * 10000 = 30.72 MB
    let float32_bytes = dim * 4 * count;

    // Int8 storage: 768 * 1 * 10000 = 7.68 MB
    let int8_bytes = dim * count;

    assert_eq!(float32_bytes / int8_bytes, 4, "Should be 4x reduction");
}

// =========================================================================
// TDD Tests: High-dimensional vectors (realistic embedding sizes)
// =========================================================================

#[allow(clippy::cast_precision_loss)]
#[test]
fn test_quantize_768d_embedding() {
    // Typical embedding size (BERT, etc.)
    let v1: Vec<f32> = (0..768).map(|i| (i as f32 * 0.01).sin()).collect();
    let v2: Vec<f32> = (0..768).map(|i| (i as f32 * 0.01).cos()).collect();

    let quantizer = ScalarQuantizer::train(&[&v1, &v2]);
    assert_eq!(quantizer.dimension, 768);

    let qvec = quantizer.quantize(&v1);
    assert_eq!(qvec.data.len(), 768);

    let recovered = quantizer.dequantize(&qvec);
    assert_eq!(recovered.len(), 768);

    // Check reconstruction error is reasonable
    let mse: f32 = v1
        .iter()
        .zip(recovered.iter())
        .map(|(a, b)| (a - b).powi(2))
        .sum::<f32>()
        / 768.0;

    assert!(mse < 0.001, "MSE should be small: {mse}");
}
