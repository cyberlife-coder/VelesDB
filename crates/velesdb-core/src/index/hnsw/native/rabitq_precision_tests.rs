//! Tests for `RaBitQPrecisionHnsw`.

use super::distance::CachedSimdDistance;
use super::rabitq_precision::{RaBitQPrecisionConfig, RaBitQPrecisionHnsw};
use crate::distance::DistanceMetric;

/// Config that forces the binary path on small test indexes (the default
/// `min_index_size` of 5000 would route them to the exact-f32 fallback).
fn binary_path_config() -> RaBitQPrecisionConfig {
    RaBitQPrecisionConfig {
        min_index_size: 0,
        ..RaBitQPrecisionConfig::default()
    }
}

// =========================================================================
// Basic lifecycle tests
// =========================================================================

#[test]
fn test_rabitq_precision_empty_index() {
    let engine = CachedSimdDistance::new(DistanceMetric::Euclidean, 64);
    let hnsw = RaBitQPrecisionHnsw::new(engine, 64, 16, 100, 1000).expect("test");

    assert!(hnsw.is_empty());
    assert!(!hnsw.is_quantizer_trained());

    let query = vec![0.0_f32; 64];
    let results = hnsw.search(&query, 10, 50);
    assert!(results.is_empty());
}

#[test]
fn test_rabitq_precision_fallback_when_untrained() {
    let engine = CachedSimdDistance::new(DistanceMetric::Euclidean, 32);
    let hnsw = RaBitQPrecisionHnsw::new(engine, 32, 16, 100, 1000).expect("test");

    // Insert fewer vectors than training threshold
    for i in 0..50 {
        let v: Vec<f32> = (0..32).map(|j| (i * 32 + j) as f32).collect();
        hnsw.insert(&v).expect("test");
    }

    assert_eq!(hnsw.len(), 50);
    assert!(!hnsw.is_quantizer_trained(), "Should not train yet");

    // Search should work via f32 fallback
    let query: Vec<f32> = (0..32).map(|j| j as f32).collect();
    let results = hnsw.search(&query, 10, 50);

    assert!(!results.is_empty());
    assert_eq!(results[0].0, 0, "Closest should be node 0");
}

#[test]
fn test_rabitq_precision_insert_trains_lazily() {
    let engine = CachedSimdDistance::new(DistanceMetric::Euclidean, 64);
    // training_sample_size = min(1000, 100) = 100
    let hnsw = RaBitQPrecisionHnsw::new(engine, 64, 16, 100, 100).expect("test");

    for i in 0..100 {
        let v: Vec<f32> = (0..64)
            .map(|j| ((i * 64 + j) as f32 * 0.01).sin())
            .collect();
        hnsw.insert(&v).expect("test");
    }

    assert!(
        hnsw.is_quantizer_trained(),
        "Quantizer should be trained after threshold"
    );
}

#[test]
fn test_rabitq_precision_force_train() {
    let engine = CachedSimdDistance::new(DistanceMetric::Euclidean, 64);
    let hnsw = RaBitQPrecisionHnsw::new(engine, 64, 16, 100, 1000).expect("test");

    // Insert fewer than threshold
    for i in 0..50 {
        let v: Vec<f32> = (0..64).map(|j| (i * 64 + j) as f32).collect();
        hnsw.insert(&v).expect("test");
    }

    assert!(!hnsw.is_quantizer_trained());

    hnsw.force_train_quantizer().expect("test");

    assert!(hnsw.is_quantizer_trained());
}

// =========================================================================
// Search after training
// =========================================================================

#[test]
fn test_rabitq_precision_search_after_training() {
    let engine = CachedSimdDistance::new(DistanceMetric::Euclidean, 64);
    let hnsw = RaBitQPrecisionHnsw::new(engine, 64, 16, 100, 1000).expect("test");

    for i in 0..200 {
        let v: Vec<f32> = (0..64)
            .map(|j| ((i * 64 + j) as f32 * 0.01).sin())
            .collect();
        hnsw.insert(&v).expect("test");
    }

    hnsw.force_train_quantizer().expect("test");

    let query: Vec<f32> = (0..64).map(|j| (j as f32 * 0.01).sin()).collect();
    let results = hnsw.search_with_config(&query, 10, 50, &binary_path_config());

    assert!(!results.is_empty());

    // Results should be sorted by distance
    for i in 1..results.len() {
        assert!(
            results[i].1 >= results[i - 1].1,
            "Results should be sorted by distance"
        );
    }
}

#[test]
fn test_rabitq_precision_insert_after_training() {
    let engine = CachedSimdDistance::new(DistanceMetric::Euclidean, 32);
    let hnsw = RaBitQPrecisionHnsw::new(engine, 32, 16, 100, 1000).expect("test");

    // Insert and train
    for i in 0..50 {
        let v: Vec<f32> = (0..32).map(|j| (i * 32 + j) as f32).collect();
        hnsw.insert(&v).expect("test");
    }
    hnsw.force_train_quantizer().expect("test");

    // Insert more after training — these should be encoded
    for i in 50..100 {
        let v: Vec<f32> = (0..32).map(|j| (i * 32 + j) as f32).collect();
        hnsw.insert(&v).expect("test");
    }

    assert_eq!(hnsw.len(), 100);

    let query: Vec<f32> = (0..32).map(|j| (75 * 32 + j) as f32).collect();
    let results = hnsw.search_with_config(&query, 5, 50, &binary_path_config());
    assert!(!results.is_empty());
}

// =========================================================================
// install_trained_rabitq (quantization wiring across restarts)
// =========================================================================

/// Builds `n` sinusoidal vectors of dimension `dim`.
#[cfg(feature = "persistence")]
fn sinusoidal_vectors(n: usize, dim: usize) -> Vec<Vec<f32>> {
    (0..n)
        .map(|i| {
            (0..dim)
                .map(|j| ((i * dim + j) as f32 * 0.01).sin())
                .collect()
        })
        .collect()
}

/// Installing a pre-trained quantizer must encode every existing vector
/// (store rebuilt in NodeId order) and activate RaBitQ search with recall
/// parity against the f32 baseline.
#[cfg(feature = "persistence")]
#[test]
fn test_install_trained_rabitq_encodes_existing_vectors() {
    use crate::quantization::RaBitQIndex;
    use std::collections::HashSet;
    use std::sync::Arc;

    let (dim, n, k) = (64, 200, 10);
    let engine = CachedSimdDistance::new(DistanceMetric::Euclidean, dim);
    let hnsw = RaBitQPrecisionHnsw::new(engine, dim, 16, 200, 1000).expect("test");

    let vectors = sinusoidal_vectors(n, dim);
    for v in &vectors {
        hnsw.insert(v).expect("insert");
    }
    assert!(!hnsw.is_quantizer_trained(), "below lazy-train threshold");

    let query = &vectors[42];
    let baseline: HashSet<usize> = hnsw
        .search(query, k, 100)
        .iter()
        .map(|&(id, _)| id)
        .collect();

    let rabitq = RaBitQIndex::train(&vectors, 42).expect("train");
    hnsw.install_trained_rabitq(Arc::new(rabitq))
        .expect("install");
    assert!(hnsw.is_quantizer_trained());

    let results = hnsw.search_with_config(query, k, 100, &binary_path_config());
    assert_eq!(results.len(), k);
    assert_eq!(results[0].0, 42, "self-query must return itself as top-1");

    let ids: HashSet<usize> = results.iter().map(|&(id, _)| id).collect();
    let overlap = baseline.intersection(&ids).count();
    #[allow(clippy::cast_precision_loss)]
    let recall = overlap as f64 / k as f64;
    assert!(
        recall >= 0.7,
        "RaBitQ results should overlap f32 baseline (recall sanity), got {recall:.2}"
    );
}

/// Inserts after install must stay aligned with NodeId order: the store was
/// rebuilt for nodes `0..n`, so node `n` (first post-install insert) must be
/// encoded at store position `n` and remain searchable.
#[cfg(feature = "persistence")]
#[test]
fn test_install_trained_rabitq_then_insert_keeps_alignment() {
    use crate::quantization::RaBitQIndex;
    use std::sync::Arc;

    let (dim, n) = (64, 120);
    let engine = CachedSimdDistance::new(DistanceMetric::Euclidean, dim);
    let hnsw = RaBitQPrecisionHnsw::new(engine, dim, 16, 200, 1000).expect("test");

    let vectors = sinusoidal_vectors(n + 30, dim);
    for v in &vectors[..n] {
        hnsw.insert(v).expect("insert");
    }

    let rabitq = RaBitQIndex::train(&vectors[..n], 42).expect("train");
    hnsw.install_trained_rabitq(Arc::new(rabitq))
        .expect("install");

    for v in &vectors[n..] {
        hnsw.insert(v).expect("post-install insert");
    }
    assert_eq!(hnsw.len(), n + 30);

    // Self-query on a post-install vector: top-1 must be its own node id.
    let target = n + 15;
    let results = hnsw.search_with_config(&vectors[target], 5, 100, &binary_path_config());
    assert_eq!(
        results.first().map(|&(id, _)| id),
        Some(target),
        "post-install vector must be searchable at its node id"
    );
}

// =========================================================================
// min_index_size fallback (doc contract on `RaBitQPrecisionConfig`)
// =========================================================================

/// Default `min_index_size` must match the documented threshold (5000).
#[test]
fn test_rabitq_precision_config_default_min_index_size() {
    assert_eq!(RaBitQPrecisionConfig::default().min_index_size, 5000);
}

/// Below `min_index_size`, a TRAINED index must skip the binary path and
/// return exactly the pre-training f32 results (ids and distances) — the
/// guard short-circuits before any `RaBitQ` machinery runs.
#[test]
fn test_rabitq_below_min_index_size_falls_back_to_f32() {
    let engine = CachedSimdDistance::new(DistanceMetric::Euclidean, 32);
    let hnsw = RaBitQPrecisionHnsw::new(engine, 32, 16, 100, 1000).expect("test");

    for i in 0..100 {
        let v: Vec<f32> = (0..32)
            .map(|j| ((i * 32 + j) as f32 * 0.01).sin())
            .collect();
        hnsw.insert(&v).expect("test");
    }

    let query: Vec<f32> = (0..32)
        .map(|j| ((42 * 32 + j) as f32 * 0.01).sin())
        .collect();
    let baseline = hnsw.search(&query, 10, 100);

    hnsw.force_train_quantizer().expect("test");
    assert!(hnsw.is_quantizer_trained());

    // 100 vectors < default min_index_size (5000): default-config search
    // must produce the identical exact-f32 result list.
    let fallback = hnsw.search(&query, 10, 100);
    assert_eq!(fallback, baseline, "below-min search must stay on exact f32");
}

// =========================================================================
// Recall test (EPIC-055)
// =========================================================================

/// Verifies recall@10 >= 0.95 on 10K vectors with `RaBitQ` traversal.
///
/// Uses 128-dimensional vectors with sinusoidal patterns to create a
/// realistic distribution. The oversampling ratio of 6 compensates for
/// `RaBitQ`'s coarser distance estimates vs SQ8.
#[test]
fn test_rabitq_precision_recall_above_threshold() {
    let dim = 128;
    let n = 10_000;
    let k = 10;
    let ef_search = 200;

    // Build index
    let engine = CachedSimdDistance::new(DistanceMetric::Euclidean, dim);
    let hnsw = RaBitQPrecisionHnsw::new(engine, dim, 32, 200, n).expect("test");

    let vectors: Vec<Vec<f32>> = (0..n)
        .map(|i| {
            (0..dim)
                .map(|j| ((i * dim + j) as f32 * 0.001).sin())
                .collect()
        })
        .collect();

    for v in &vectors {
        hnsw.insert(v).expect("test");
    }

    // Quantizer auto-trained after 1000 vectors; remaining 9000 encoded

    // Compute brute-force ground truth for 5 random queries
    let query_indices = [0, 1000, 5000, 7777, 9999];
    let mut total_recall = 0.0;

    for &qi in &query_indices {
        let query = &vectors[qi];

        // Brute-force top-k
        let mut brute: Vec<(usize, f32)> = vectors
            .iter()
            .enumerate()
            .map(|(idx, v)| {
                let dist: f32 = query
                    .iter()
                    .zip(v.iter())
                    .map(|(&a, &b)| (a - b) * (a - b))
                    .sum::<f32>()
                    .sqrt();
                (idx, dist)
            })
            .collect();
        brute.sort_by(|a, b| a.1.total_cmp(&b.1));
        brute.truncate(k);

        let brute_ids: std::collections::HashSet<usize> = brute.iter().map(|(id, _)| *id).collect();

        // RaBitQ-precision search
        let results = hnsw.search(query, k, ef_search);
        let result_ids: std::collections::HashSet<usize> =
            results.iter().map(|(id, _)| *id).collect();

        let overlap = brute_ids.intersection(&result_ids).count();
        #[allow(clippy::cast_precision_loss)]
        let recall = overlap as f64 / k as f64;
        total_recall += recall;
    }

    #[allow(clippy::cast_precision_loss)]
    let avg_recall = total_recall / query_indices.len() as f64;
    assert!(
        avg_recall >= 0.95,
        "RaBitQ recall@{k} should be >= 0.95, got {avg_recall:.3}"
    );
}

// =========================================================================
// Regression: rerank must sort by METRIC semantics, not ascending raw value.
// After transform_score, Cosine/DotProduct are similarities (higher =
// better); an ascending sort + truncate(k) keeps the k WORST candidates.
// =========================================================================

/// Builds a trained `RaBitQ` index over `n` unit vectors and searches with
/// the self-query, asserting top-1 identity and recall@k >= 0.95.
///
/// Uses 64 dims (vs 32 for SQ8 tests): `RaBitQ` allocates 1 bit per dim,
/// and 32-bit codes are too coarse to rank near-orthogonal random vectors.
fn run_rabitq_self_query(metric: DistanceMetric) {
    use super::dual_precision_tests::{assert_top1_and_recall, planted_unit_vectors};

    let (dim, n, k) = (64, 100, 10);
    let query_id = 42_usize;
    let engine = CachedSimdDistance::new(metric, dim);
    let hnsw = RaBitQPrecisionHnsw::new(engine, dim, 16, 200, 1000).expect("test");

    let vectors = planted_unit_vectors(n, dim, query_id);
    for v in &vectors {
        hnsw.insert(v).expect("test");
    }
    hnsw.force_train_quantizer().expect("test");
    assert!(hnsw.is_quantizer_trained());

    let results = hnsw.search_with_config(&vectors[query_id], k, 100, &binary_path_config());

    assert_top1_and_recall(&results, &vectors, query_id, metric, k);
}

#[test]
fn test_rabitq_cosine_rerank_keeps_best_candidates() {
    run_rabitq_self_query(DistanceMetric::Cosine);
}

#[test]
fn test_rabitq_dot_product_rerank_keeps_best_candidates() {
    run_rabitq_self_query(DistanceMetric::DotProduct);
}

// =========================================================================
// Regression: transform_score applied
// =========================================================================

#[test]
fn test_rabitq_euclidean_returns_sqrt_not_squared() {
    use super::distance::CachedSimdDistance;

    let dim = 32;
    let engine = CachedSimdDistance::new(DistanceMetric::Euclidean, dim);
    let hnsw = RaBitQPrecisionHnsw::new(engine, dim, 16, 100, 1000).expect("test");

    let v0 = vec![0.0_f32; dim];
    let v1 = vec![1.0_f32; dim];
    hnsw.insert(&v0).expect("test");
    hnsw.insert(&v1).expect("test");

    hnsw.force_train_quantizer().expect("test");

    let results = hnsw.search_with_config(&v0, 2, 50, &binary_path_config());
    assert!(
        results.len() >= 2,
        "Expected at least 2 results, got {}",
        results.len()
    );

    let v1_dist = results
        .iter()
        .find(|(id, _)| *id == 1)
        .map(|(_, d)| *d)
        .expect("v1 should be in results");

    let expected = (dim as f32).sqrt();
    let tolerance = 0.01;

    assert!(
        (v1_dist - expected).abs() < tolerance,
        "Distance to v1 should be sqrt({dim}) ~= {expected:.3}, got {v1_dist:.3}"
    );
}
