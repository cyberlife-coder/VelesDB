//! Tests for `dual_precision` module

use super::distance::SimdDistance;
use super::dual_precision::*;
use crate::distance::DistanceMetric;

// =========================================================================
// TDD Tests: DualPrecisionHnsw creation and basic operations
// =========================================================================

#[test]
fn test_create_dual_precision_hnsw() {
    let engine = SimdDistance::new(DistanceMetric::Euclidean);
    let hnsw = DualPrecisionHnsw::new(engine, 128, 16, 100, 1000);

    assert!(hnsw.is_empty());
    assert!(!hnsw.is_quantizer_trained());
}

#[test]
fn test_insert_before_quantizer_training() {
    let engine = SimdDistance::new(DistanceMetric::Euclidean);
    let mut hnsw = DualPrecisionHnsw::new(engine, 32, 16, 100, 1000);

    // Insert fewer vectors than training threshold
    for i in 0..10 {
        let v: Vec<f32> = (0..32).map(|j| (i * 32 + j) as f32).collect();
        hnsw.insert(v);
    }

    assert_eq!(hnsw.len(), 10);
    assert!(!hnsw.is_quantizer_trained(), "Should not train yet");
}

#[test]
fn test_quantizer_trains_after_threshold() {
    let engine = SimdDistance::new(DistanceMetric::Euclidean);
    // Set low training threshold for test
    let mut hnsw = DualPrecisionHnsw::new(engine, 32, 16, 100, 100);
    // training_sample_size = min(1000, 100) = 100

    // Insert up to threshold
    for i in 0..100 {
        let v: Vec<f32> = (0..32)
            .map(|j| ((i * 32 + j) as f32 * 0.01).sin())
            .collect();
        hnsw.insert(v);
    }

    assert!(
        hnsw.is_quantizer_trained(),
        "Quantizer should be trained after threshold"
    );
}

#[test]
fn test_force_train_quantizer() {
    let engine = SimdDistance::new(DistanceMetric::Euclidean);
    let mut hnsw = DualPrecisionHnsw::new(engine, 32, 16, 100, 1000);

    // Insert fewer than threshold
    for i in 0..50 {
        let v: Vec<f32> = (0..32).map(|j| (i * 32 + j) as f32).collect();
        hnsw.insert(v);
    }

    assert!(!hnsw.is_quantizer_trained());

    // Force training
    hnsw.force_train_quantizer();

    assert!(hnsw.is_quantizer_trained());
}

// =========================================================================
// TDD Tests: Search behavior
// =========================================================================

#[test]
fn test_search_before_quantizer_training() {
    let engine = SimdDistance::new(DistanceMetric::Euclidean);
    let mut hnsw = DualPrecisionHnsw::new(engine, 32, 16, 100, 1000);

    // Insert some vectors
    for i in 0..50 {
        let v: Vec<f32> = (0..32).map(|j| (i * 32 + j) as f32).collect();
        hnsw.insert(v);
    }

    // Search without quantizer (should use float32)
    let query: Vec<f32> = (0..32).map(|j| j as f32).collect();
    let results = hnsw.search(&query, 10, 50);

    assert!(!results.is_empty());
    // First result should be node 0 (closest to query)
    assert_eq!(results[0].0, 0);
}

#[test]
fn test_search_after_quantizer_training() {
    let engine = SimdDistance::new(DistanceMetric::Euclidean);
    let mut hnsw = DualPrecisionHnsw::new(engine, 32, 16, 100, 1000);

    // Insert vectors
    for i in 0..50 {
        let v: Vec<f32> = (0..32).map(|j| (i * 32 + j) as f32).collect();
        hnsw.insert(v);
    }

    // Force train quantizer
    hnsw.force_train_quantizer();

    // Search with dual-precision
    let query: Vec<f32> = (0..32).map(|j| j as f32).collect();
    let results = hnsw.search(&query, 10, 50);

    assert!(!results.is_empty());
    // First result should still be node 0
    assert_eq!(results[0].0, 0);
}

#[test]
fn test_dual_precision_recall() {
    let engine = SimdDistance::new(DistanceMetric::Euclidean);
    let mut hnsw = DualPrecisionHnsw::new(engine, 128, 32, 200, 1000);

    // Insert 200 vectors
    let vectors: Vec<Vec<f32>> = (0..200)
        .map(|i| {
            (0..128)
                .map(|j| ((i * 128 + j) as f32 * 0.01).sin())
                .collect()
        })
        .collect();

    for v in &vectors {
        hnsw.insert(v.clone());
    }

    hnsw.force_train_quantizer();

    // Search
    let query: Vec<f32> = (0..128).map(|j| (j as f32 * 0.01).sin()).collect();
    let results = hnsw.search(&query, 10, 100);

    assert!(results.len() >= 5, "Should find at least 5 neighbors");

    // Results should be sorted by distance
    for i in 1..results.len() {
        assert!(
            results[i].1 >= results[i - 1].1,
            "Results should be sorted by distance"
        );
    }
}

// =========================================================================
// TDD Tests: Insert after quantizer training
// =========================================================================

#[test]
fn test_insert_after_quantizer_training() {
    let engine = SimdDistance::new(DistanceMetric::Euclidean);
    let mut hnsw = DualPrecisionHnsw::new(engine, 32, 16, 100, 1000);

    // Insert and train
    for i in 0..50 {
        let v: Vec<f32> = (0..32).map(|j| (i * 32 + j) as f32).collect();
        hnsw.insert(v);
    }
    hnsw.force_train_quantizer();

    // Insert more after training
    for i in 50..100 {
        let v: Vec<f32> = (0..32).map(|j| (i * 32 + j) as f32).collect();
        hnsw.insert(v);
    }

    assert_eq!(hnsw.len(), 100);

    // Search should find vectors from both phases
    let query: Vec<f32> = (0..32).map(|j| (75 * 32 + j) as f32).collect();
    let results = hnsw.search(&query, 5, 50);

    assert!(!results.is_empty());
}

// =========================================================================
// TDD Tests: Quantized reranking optimization (US-003)
// =========================================================================

#[test]
fn test_quantized_reranking_uses_asymmetric_distance() {
    let engine = SimdDistance::new(DistanceMetric::Euclidean);
    let mut hnsw = DualPrecisionHnsw::new(engine, 64, 16, 100, 500);

    // Insert 200 vectors
    for i in 0..200 {
        let v: Vec<f32> = (0..64)
            .map(|j| ((i * 64 + j) as f32 * 0.01).sin())
            .collect();
        hnsw.insert(v);
    }

    // Force train quantizer
    hnsw.force_train_quantizer();
    assert!(hnsw.is_quantizer_trained());

    // Search should use quantized reranking
    let query: Vec<f32> = (0..64).map(|j| (j as f32 * 0.01).sin()).collect();
    let results = hnsw.search(&query, 10, 50);

    assert!(!results.is_empty());
    // Results should be properly sorted by exact distance
    for i in 1..results.len() {
        assert!(
            results[i].1 >= results[i - 1].1,
            "Results must be sorted by exact distance after reranking"
        );
    }
}

#[test]
fn test_quantized_reranking_maintains_recall() {
    let engine = SimdDistance::new(DistanceMetric::Euclidean);
    let mut hnsw = DualPrecisionHnsw::new(engine, 128, 32, 200, 1000);

    // Insert 500 vectors
    let vectors: Vec<Vec<f32>> = (0..500)
        .map(|i| {
            (0..128)
                .map(|j| ((i * 128 + j) as f32 * 0.001).cos())
                .collect()
        })
        .collect();

    for v in &vectors {
        hnsw.insert(v.clone());
    }

    hnsw.force_train_quantizer();

    // Search with known query (should find exact match at index 0)
    let query = vectors[0].clone();
    let results = hnsw.search(&query, 10, 100);

    // Node 0 should be in top results (recall check)
    let found_exact = results.iter().any(|(id, _)| *id == 0);
    assert!(
        found_exact,
        "Quantized reranking should maintain high recall"
    );
}
