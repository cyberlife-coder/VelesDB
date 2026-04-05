//! Tests for `dual_precision` module

use super::distance::CachedSimdDistance;
use super::dual_precision::{DualPrecisionConfig, DualPrecisionHnsw};
use crate::distance::DistanceMetric;

// =========================================================================
// TDD Tests: DualPrecisionHnsw creation and basic operations
// =========================================================================

#[test]
fn test_create_dual_precision_hnsw() {
    let engine = CachedSimdDistance::new(DistanceMetric::Euclidean, 128);
    let hnsw = DualPrecisionHnsw::new(engine, 128, 16, 100, 1000).expect("test");

    assert!(hnsw.is_empty());
    assert!(!hnsw.is_quantizer_trained());
}

#[test]
fn test_insert_before_quantizer_training() {
    let engine = CachedSimdDistance::new(DistanceMetric::Euclidean, 32);
    let mut hnsw = DualPrecisionHnsw::new(engine, 32, 16, 100, 1000).expect("test");

    // Insert fewer vectors than training threshold
    for i in 0..10 {
        let v: Vec<f32> = (0..32).map(|j| (i * 32 + j) as f32).collect();
        hnsw.insert(&v).expect("test");
    }

    assert_eq!(hnsw.len(), 10);
    assert!(!hnsw.is_quantizer_trained(), "Should not train yet");
}

#[test]
fn test_quantizer_trains_after_threshold() {
    let engine = CachedSimdDistance::new(DistanceMetric::Euclidean, 32);
    // Set low training threshold for test
    let mut hnsw = DualPrecisionHnsw::new(engine, 32, 16, 100, 100).expect("test");
    // training_sample_size = min(1000, 100) = 100

    // Insert up to threshold
    for i in 0..100 {
        let v: Vec<f32> = (0..32)
            .map(|j| ((i * 32 + j) as f32 * 0.01).sin())
            .collect();
        hnsw.insert(&v).expect("test");
    }

    assert!(
        hnsw.is_quantizer_trained(),
        "Quantizer should be trained after threshold"
    );
}

#[test]
fn test_force_train_quantizer() {
    let engine = CachedSimdDistance::new(DistanceMetric::Euclidean, 32);
    let mut hnsw = DualPrecisionHnsw::new(engine, 32, 16, 100, 1000).expect("test");

    // Insert fewer than threshold
    for i in 0..50 {
        let v: Vec<f32> = (0..32).map(|j| (i * 32 + j) as f32).collect();
        hnsw.insert(&v).expect("test");
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
    let engine = CachedSimdDistance::new(DistanceMetric::Euclidean, 32);
    let mut hnsw = DualPrecisionHnsw::new(engine, 32, 16, 100, 1000).expect("test");

    // Insert some vectors
    for i in 0..50 {
        let v: Vec<f32> = (0..32).map(|j| (i * 32 + j) as f32).collect();
        hnsw.insert(&v).expect("test");
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
    let engine = CachedSimdDistance::new(DistanceMetric::Euclidean, 32);
    let mut hnsw = DualPrecisionHnsw::new(engine, 32, 16, 100, 1000).expect("test");

    // Insert vectors
    for i in 0..50 {
        let v: Vec<f32> = (0..32).map(|j| (i * 32 + j) as f32).collect();
        hnsw.insert(&v).expect("test");
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
    let engine = CachedSimdDistance::new(DistanceMetric::Euclidean, 128);
    let mut hnsw = DualPrecisionHnsw::new(engine, 128, 32, 200, 1000).expect("test");

    // Insert 200 vectors
    let vectors: Vec<Vec<f32>> = (0..200)
        .map(|i| {
            (0..128)
                .map(|j| ((i * 128 + j) as f32 * 0.01).sin())
                .collect()
        })
        .collect();

    for v in &vectors {
        hnsw.insert(v).expect("test");
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
    let engine = CachedSimdDistance::new(DistanceMetric::Euclidean, 32);
    let mut hnsw = DualPrecisionHnsw::new(engine, 32, 16, 100, 1000).expect("test");

    // Insert and train
    for i in 0..50 {
        let v: Vec<f32> = (0..32).map(|j| (i * 32 + j) as f32).collect();
        hnsw.insert(&v).expect("test");
    }
    hnsw.force_train_quantizer();

    // Insert more after training
    for i in 50..100 {
        let v: Vec<f32> = (0..32).map(|j| (i * 32 + j) as f32).collect();
        hnsw.insert(&v).expect("test");
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
    let engine = CachedSimdDistance::new(DistanceMetric::Euclidean, 64);
    let mut hnsw = DualPrecisionHnsw::new(engine, 64, 16, 100, 500).expect("test");

    // Insert 200 vectors
    for i in 0..200 {
        let v: Vec<f32> = (0..64)
            .map(|j| ((i * 64 + j) as f32 * 0.01).sin())
            .collect();
        hnsw.insert(&v).expect("test");
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
    let engine = CachedSimdDistance::new(DistanceMetric::Euclidean, 128);
    let mut hnsw = DualPrecisionHnsw::new(engine, 128, 32, 200, 1000).expect("test");

    // Insert 500 vectors
    let vectors: Vec<Vec<f32>> = (0..500)
        .map(|i| {
            (0..128)
                .map(|j| ((i * 128 + j) as f32 * 0.001).cos())
                .collect()
        })
        .collect();

    for v in &vectors {
        hnsw.insert(v).expect("test");
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

// =========================================================================
// TDD Tests: TRUE int8 traversal (EPIC-055/US-003 requirement)
// =========================================================================

#[test]
fn test_search_with_int8_traversal_enabled() {
    let engine = CachedSimdDistance::new(DistanceMetric::Euclidean, 64);
    let mut hnsw = DualPrecisionHnsw::new(engine, 64, 16, 100, 500).expect("test");

    // Insert vectors
    for i in 0..200 {
        let v: Vec<f32> = (0..64)
            .map(|j| ((i * 64 + j) as f32 * 0.01).sin())
            .collect();
        hnsw.insert(&v).expect("test");
    }

    hnsw.force_train_quantizer();

    // Search with TRUE int8 traversal
    let query: Vec<f32> = (0..64).map(|j| (j as f32 * 0.01).sin()).collect();
    let config = DualPrecisionConfig {
        oversampling_ratio: 4,
        use_int8_traversal: true, // Force int8 graph traversal
        ..Default::default()
    };
    let results = hnsw.search_with_config(&query, 10, 50, &config);

    assert!(!results.is_empty());
    // Results should be sorted by distance
    for i in 1..results.len() {
        assert!(results[i].1 >= results[i - 1].1, "Results should be sorted");
    }
}

#[test]
fn test_int8_traversal_recall_vs_f32() {
    let engine = CachedSimdDistance::new(DistanceMetric::Euclidean, 128);
    let mut hnsw = DualPrecisionHnsw::new(engine, 128, 32, 200, 1000).expect("test");

    // Insert 500 vectors
    let vectors: Vec<Vec<f32>> = (0..500)
        .map(|i| {
            (0..128)
                .map(|j| ((i * 128 + j) as f32 * 0.001).cos())
                .collect()
        })
        .collect();

    for v in &vectors {
        hnsw.insert(v).expect("test");
    }

    hnsw.force_train_quantizer();

    // Search with f32 (baseline)
    let query = vectors[0].clone();
    let f32_results = hnsw.search(&query, 10, 100);

    // Search with int8 traversal
    let config = DualPrecisionConfig {
        oversampling_ratio: 4,
        use_int8_traversal: true,
        ..Default::default()
    };
    let int8_results = hnsw.search_with_config(&query, 10, 100, &config);

    // Compute recall: how many of f32 results are in int8 results
    let f32_ids: std::collections::HashSet<_> = f32_results.iter().map(|(id, _)| *id).collect();
    let int8_ids: std::collections::HashSet<_> = int8_results.iter().map(|(id, _)| *id).collect();
    let overlap = f32_ids.intersection(&int8_ids).count();
    let recall = overlap as f64 / f32_results.len().max(1) as f64;

    // Recall should be >= 90% (int8 traversal with 4x oversampling)
    assert!(
        recall >= 0.90,
        "Int8 traversal recall should be >= 90%, got {:.2}%",
        recall * 100.0
    );
}

#[test]
fn test_dual_precision_config_defaults() {
    let config = DualPrecisionConfig::default();
    assert_eq!(config.oversampling_ratio, 4);
    assert!(config.use_int8_traversal);
    assert_eq!(config.min_index_size, 10_000);
}

// =========================================================================
// Regression: rerank_with_exact_f32 applies transform_score (C-2 / #420)
// =========================================================================

/// Verifies that `DualPrecisionHnsw` with `CachedSimdDistance` (production
/// engine) returns actual Euclidean distances (with sqrt), NOT squared L2.
///
/// Before the fix, `rerank_with_exact_f32` returned raw `compute_distance()`
/// values which are squared L2 for Euclidean under `CachedSimdDistance`.
#[test]
fn test_rerank_euclidean_returns_sqrt_not_squared_with_cached_engine() {
    use super::distance::CachedSimdDistance;

    let dim = 32;
    let engine = CachedSimdDistance::new(DistanceMetric::Euclidean, dim);
    let mut hnsw = DualPrecisionHnsw::new(engine, dim, 16, 100, 1000).expect("test");

    // Insert two known vectors: origin and a vector at distance 1.0 per component
    // v0 = [0, 0, 0, ...]
    // v1 = [1, 1, 1, ...]
    // Expected Euclidean distance from v0 to v1 = sqrt(32 * 1^2) = sqrt(32) ~= 5.657
    // Squared L2 would be 32.0 (the bug value)
    let v0 = vec![0.0_f32; dim];
    let v1 = vec![1.0_f32; dim];
    hnsw.insert(&v0).expect("test");
    hnsw.insert(&v1).expect("test");

    // Force-train to enable dual-precision search path
    hnsw.force_train_quantizer();

    // Search from v0 — expect both v0 (dist=0) and v1 (dist=sqrt(32))
    let results = hnsw.search(&v0, 2, 50);
    assert!(
        results.len() >= 2,
        "Expected at least 2 results, got {}",
        results.len()
    );

    // Find v1's distance in results
    let v1_dist = results
        .iter()
        .find(|(id, _)| *id == 1)
        .map(|(_, d)| *d)
        .expect("v1 should be in results");

    let expected = (dim as f32).sqrt(); // sqrt(32) ~= 5.657
    let tolerance = 0.01;

    // This assertion would fail pre-fix: v1_dist would be 32.0 (squared L2)
    assert!(
        (v1_dist - expected).abs() < tolerance,
        "Distance to v1 should be sqrt({dim}) ~= {expected:.3}, got {v1_dist:.3} \
         (if ~{dim}.0, transform_score was not applied)"
    );
}

/// Same regression test for Cosine metric — verifies transform_score clamps
/// cosine similarity correctly through the dual-precision rerank path.
#[test]
fn test_rerank_cosine_applies_transform_with_cached_engine() {
    use super::distance::CachedSimdDistance;

    let dim = 32;
    let engine = CachedSimdDistance::new(DistanceMetric::Cosine, dim);
    let mut hnsw = DualPrecisionHnsw::new(engine, dim, 16, 100, 1000).expect("test");

    // Insert normalized vectors
    let norm = 1.0 / (dim as f32).sqrt();
    let v0: Vec<f32> = vec![norm; dim];
    // v1 is orthogonal-ish to v0
    let mut v1 = vec![0.0_f32; dim];
    let v1_norm = 1.0 / (dim as f32 / 2.0).sqrt();
    for slot in v1.iter_mut().take(dim / 2) {
        *slot = v1_norm;
    }

    hnsw.insert(&v0).expect("test");
    hnsw.insert(&v1).expect("test");

    hnsw.force_train_quantizer();

    let results = hnsw.search(&v0, 2, 50);
    assert!(!results.is_empty());

    // All cosine scores should be in [0, 1] after transform_score clamping
    for (id, score) in &results {
        assert!(
            *score >= 0.0 && *score <= 1.0,
            "Cosine score for node {id} should be in [0,1], got {score}"
        );
    }
}
