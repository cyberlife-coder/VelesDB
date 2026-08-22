//! Tests for `Sq8PrecisionHnsw` (int8 traversal + f32 re-ranking).
//!
//! Ports the `DualPrecisionHnsw` prototype's behavior pins onto the wired
//! codec-generic backend, plus the new contracts: interior-mutability
//! inserts, metric gating, and `install_trained_sq8` alignment.

use super::distance::CachedSimdDistance;
use super::precision_test_support::{assert_top1_and_recall, planted_unit_vectors};
use super::sq8_precision::{Sq8PrecisionConfig, Sq8PrecisionHnsw};
use crate::distance::DistanceMetric;

/// Config that forces the int8 path on small test indexes (the default
/// `min_index_size` of 10 000 would route them to the exact-f32 fallback).
fn int8_path_config() -> Sq8PrecisionConfig {
    Sq8PrecisionConfig {
        min_index_size: 0,
        ..Sq8PrecisionConfig::default()
    }
}

// =========================================================================
// Basic lifecycle
// =========================================================================

#[test]
fn test_create_sq8_precision_hnsw() {
    let engine = CachedSimdDistance::new(DistanceMetric::Euclidean, 128);
    let hnsw = Sq8PrecisionHnsw::new(engine, 128, 16, 100, 1000).expect("test");

    assert!(hnsw.is_empty());
    assert!(!hnsw.is_quantizer_trained());
}

#[test]
fn test_insert_before_quantizer_training() {
    let engine = CachedSimdDistance::new(DistanceMetric::Euclidean, 32);
    let hnsw = Sq8PrecisionHnsw::new(engine, 32, 16, 100, 1000).expect("test");

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
    // training_sample_size = min(1000, 100) = 100
    let hnsw = Sq8PrecisionHnsw::new(engine, 32, 16, 100, 100).expect("test");

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
    let hnsw = Sq8PrecisionHnsw::new(engine, 32, 16, 100, 1000).expect("test");

    for i in 0..50 {
        let v: Vec<f32> = (0..32).map(|j| (i * 32 + j) as f32).collect();
        hnsw.insert(&v).expect("test");
    }

    assert!(!hnsw.is_quantizer_trained());

    hnsw.force_train_quantizer().expect("test");

    assert!(hnsw.is_quantizer_trained());
}

// =========================================================================
// Search behavior
// =========================================================================

#[test]
fn test_search_before_quantizer_training() {
    let engine = CachedSimdDistance::new(DistanceMetric::Euclidean, 32);
    let hnsw = Sq8PrecisionHnsw::new(engine, 32, 16, 100, 1000).expect("test");

    for i in 0..50 {
        let v: Vec<f32> = (0..32).map(|j| (i * 32 + j) as f32).collect();
        hnsw.insert(&v).expect("test");
    }

    // Search without quantizer (should use float32)
    let query: Vec<f32> = (0..32).map(|j| j as f32).collect();
    let results = hnsw.search(&query, 10, 50);

    assert!(!results.is_empty());
    assert_eq!(results[0].0, 0, "First result should be node 0");
}

#[test]
fn test_int8_search_after_quantizer_training() {
    let engine = CachedSimdDistance::new(DistanceMetric::Euclidean, 32);
    let hnsw = Sq8PrecisionHnsw::new(engine, 32, 16, 100, 1000).expect("test");

    for i in 0..50 {
        let v: Vec<f32> = (0..32).map(|j| (i * 32 + j) as f32).collect();
        hnsw.insert(&v).expect("test");
    }

    hnsw.force_train_quantizer().expect("test");

    let query: Vec<f32> = (0..32).map(|j| j as f32).collect();
    let results = hnsw.search_with_config(&query, 10, 50, &int8_path_config());

    assert!(!results.is_empty());
    assert_eq!(results[0].0, 0, "First result should still be node 0");
}

#[test]
fn test_insert_after_quantizer_training() {
    let engine = CachedSimdDistance::new(DistanceMetric::Euclidean, 32);
    let hnsw = Sq8PrecisionHnsw::new(engine, 32, 16, 100, 1000).expect("test");

    // Insert and train
    for i in 0..50 {
        let v: Vec<f32> = (0..32).map(|j| (i * 32 + j) as f32).collect();
        hnsw.insert(&v).expect("test");
    }
    hnsw.force_train_quantizer().expect("test");

    // Insert more after training — these are encoded into the store
    for i in 50..100 {
        let v: Vec<f32> = (0..32).map(|j| (i * 32 + j) as f32).collect();
        hnsw.insert(&v).expect("test");
    }

    assert_eq!(hnsw.len(), 100);

    let query: Vec<f32> = (0..32).map(|j| (75 * 32 + j) as f32).collect();
    let results = hnsw.search_with_config(&query, 5, 50, &int8_path_config());
    assert_eq!(
        results[0].0, 75,
        "top-1 must be node 75 (post-training insert whose vector exactly matches the query) — \
         a misaligned store slot would surface the wrong node"
    );
}

// =========================================================================
// Metric gating: int8 L2 traversal only where its ordering is sound
// =========================================================================

/// On an unsupported metric (`DotProduct`: L2 ordering diverges from inner
/// product on unnormalized data) the backend must stay a plain f32
/// pass-through: no training, and search identical to the exact path.
#[test]
fn test_sq8_unsupported_metric_stays_exact_f32() {
    let engine = CachedSimdDistance::new(DistanceMetric::DotProduct, 32);
    let hnsw = Sq8PrecisionHnsw::new(engine, 32, 16, 100, 100).expect("test");

    // Past the lazy-train threshold — a supported metric would have trained.
    for i in 0..100 {
        let v: Vec<f32> = (0..32)
            .map(|j| ((i * 32 + j) as f32 * 0.01).sin())
            .collect();
        hnsw.insert(&v).expect("test");
    }
    assert!(
        !hnsw.is_quantizer_trained(),
        "unsupported metric must never train a quantizer"
    );
    hnsw.force_train_quantizer().expect("test");
    assert!(
        !hnsw.is_quantizer_trained(),
        "force_train must stay a no-op on an unsupported metric"
    );

    // Even a forced-int8 config must produce the exact-f32 result list.
    // (No top-1 identity assertion: dot product is norm-sensitive, so a
    // self-query has no guaranteed rank — result-list equality is the pin.)
    let query: Vec<f32> = (0..32)
        .map(|j| ((42 * 32 + j) as f32 * 0.01).sin())
        .collect();
    let exact = hnsw.search(&query, 10, 100);
    let forced = hnsw.search_with_config(&query, 10, 100, &int8_path_config());
    assert!(!exact.is_empty(), "exact search must return results");
    assert_eq!(forced, exact, "unsupported metric must stay on exact f32");
}

/// `install_trained_sq8` must refuse (return false) on an unsupported
/// metric instead of installing a store the traversal would misuse.
#[test]
fn test_sq8_install_refused_on_unsupported_metric() {
    use super::quantization::ScalarQuantizer;
    use std::sync::Arc;

    let engine = CachedSimdDistance::new(DistanceMetric::Hamming, 8);
    let hnsw = Sq8PrecisionHnsw::new(engine, 8, 16, 100, 1000).expect("test");
    for i in 0..10 {
        let v: Vec<f32> = (0..8).map(|j| ((i + j) % 2) as f32).collect();
        hnsw.insert(&v).expect("test");
    }

    let samples: Vec<Vec<f32>> = vec![vec![0.0; 8], vec![1.0; 8]];
    let refs: Vec<&[f32]> = samples.iter().map(Vec::as_slice).collect();
    let quantizer = Arc::new(ScalarQuantizer::train(&refs).expect("train"));
    let installed = hnsw.install_trained_sq8(quantizer).expect("install call");
    assert!(!installed, "install must be refused on Hamming");
    assert!(!hnsw.is_quantizer_trained());
}

// =========================================================================
// install_trained_sq8 (persistence wiring across restarts)
// =========================================================================

/// Builds `n` sinusoidal vectors of dimension `dim`.
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
/// (store rebuilt in NodeId order) and activate int8 search with recall
/// parity against the f32 baseline.
#[test]
fn test_install_trained_sq8_encodes_existing_vectors() {
    use super::quantization::ScalarQuantizer;
    use std::collections::HashSet;
    use std::sync::Arc;

    let (dim, n, k) = (64, 200, 10);
    let engine = CachedSimdDistance::new(DistanceMetric::Euclidean, dim);
    let hnsw = Sq8PrecisionHnsw::new(engine, dim, 16, 200, 1000).expect("test");

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

    let refs: Vec<&[f32]> = vectors.iter().map(Vec::as_slice).collect();
    let quantizer = ScalarQuantizer::train(&refs).expect("train");
    let installed = hnsw
        .install_trained_sq8(Arc::new(quantizer))
        .expect("install");
    assert!(installed);
    assert!(hnsw.is_quantizer_trained());

    let results = hnsw.search_with_config(query, k, 100, &int8_path_config());
    assert_eq!(results.len(), k);
    assert_eq!(results[0].0, 42, "self-query must return itself as top-1");

    let ids: HashSet<usize> = results.iter().map(|&(id, _)| id).collect();
    let overlap = baseline.intersection(&ids).count();
    #[allow(clippy::cast_precision_loss)]
    let recall = overlap as f64 / k as f64;
    assert!(
        recall >= 0.9,
        "int8 results should overlap f32 baseline (recall sanity), got {recall:.2}"
    );
}

/// Inserts after install must stay aligned with NodeId order: the store was
/// rebuilt for nodes `0..n`, so node `n` (first post-install insert) must be
/// encoded at store position `n` and remain searchable.
#[test]
fn test_install_trained_sq8_then_insert_keeps_alignment() {
    use super::quantization::ScalarQuantizer;
    use std::sync::Arc;

    let (dim, n) = (64, 120);
    let engine = CachedSimdDistance::new(DistanceMetric::Euclidean, dim);
    let hnsw = Sq8PrecisionHnsw::new(engine, dim, 16, 200, 1000).expect("test");

    let vectors = sinusoidal_vectors(n + 30, dim);
    for v in &vectors[..n] {
        hnsw.insert(v).expect("insert");
    }

    let refs: Vec<&[f32]> = vectors[..n].iter().map(Vec::as_slice).collect();
    let quantizer = ScalarQuantizer::train(&refs).expect("train");
    hnsw.install_trained_sq8(Arc::new(quantizer))
        .expect("install");

    for v in &vectors[n..] {
        hnsw.insert(v).expect("post-install insert");
    }
    assert_eq!(hnsw.len(), n + 30);

    // Self-query on a post-install vector: top-1 must be its own node id.
    let target = n + 15;
    let results = hnsw.search_with_config(&vectors[target], 5, 100, &int8_path_config());
    assert_eq!(
        results.first().map(|&(id, _)| id),
        Some(target),
        "post-install vector must be searchable at its node id"
    );
}

// =========================================================================
// min_index_size fallback (doc contract on `Sq8PrecisionConfig`)
// =========================================================================

/// Defaults must match the documented contract (EPIC-055/US-003).
#[test]
fn test_sq8_precision_config_defaults() {
    let config = Sq8PrecisionConfig::default();
    assert_eq!(config.oversampling_ratio, 4);
    assert_eq!(config.min_index_size, 10_000);

    // Behavioral guard: with a small index (< default min_index_size),
    // default-config search must take the f32 fallback and thus match the
    // exact path bit-for-bit.
    let engine = CachedSimdDistance::new(DistanceMetric::Euclidean, 32);
    let hnsw = Sq8PrecisionHnsw::new(engine, 32, 16, 100, 1000).expect("test");
    for i in 0..50 {
        let v: Vec<f32> = (0..32).map(|j| (i * 32 + j) as f32).collect();
        hnsw.insert(&v).expect("test");
    }
    hnsw.force_train_quantizer().expect("test");
    let query: Vec<f32> = (0..32).map(|j| j as f32).collect();
    let with_default = hnsw.search_with_config(&query, 10, 50, &Sq8PrecisionConfig::default());
    let plain = hnsw.search(&query, 10, 50);
    assert_eq!(
        with_default, plain,
        "default min_index_size=10_000 must force small indexes onto the f32 fallback path"
    );
}

// =========================================================================
// Rerank semantics: transform_score + metric ordering
// =========================================================================

/// The rerank path must return actual Euclidean distances (with sqrt), NOT
/// the engine's raw squared L2.
#[test]
fn test_rerank_euclidean_returns_sqrt_not_squared() {
    let dim = 32;
    let engine = CachedSimdDistance::new(DistanceMetric::Euclidean, dim);
    let hnsw = Sq8PrecisionHnsw::new(engine, dim, 16, 100, 1000).expect("test");

    // v0 = origin, v1 = ones: Euclidean distance sqrt(32) ~= 5.657
    // (squared L2 would be 32.0 — the bug value).
    let v0 = vec![0.0_f32; dim];
    let v1 = vec![1.0_f32; dim];
    hnsw.insert(&v0).expect("test");
    hnsw.insert(&v1).expect("test");

    hnsw.force_train_quantizer().expect("test");

    let results = hnsw.search_with_config(&v0, 2, 50, &int8_path_config());
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
    assert!(
        (v1_dist - expected).abs() < 0.01,
        "Distance to v1 should be sqrt({dim}) ~= {expected:.3}, got {v1_dist:.3} \
         (if ~{dim}.0, transform_score was not applied)"
    );
}

/// Cosine through the int8 path: the query and the codes must both live in
/// the normalized (stored) vector space, the rerank must sort by similarity
/// (higher = better), and scores must be clamped to [0, 1].
#[test]
fn test_int8_cosine_rerank_keeps_best_candidates() {
    let (dim, n, k) = (32, 100, 10);
    let query_id = 42_usize;
    let engine = CachedSimdDistance::new(DistanceMetric::Cosine, dim);
    let hnsw = Sq8PrecisionHnsw::new(engine, dim, 16, 200, 1000).expect("test");

    let vectors = planted_unit_vectors(n, dim, query_id);
    for v in &vectors {
        hnsw.insert(v).expect("test");
    }
    hnsw.force_train_quantizer().expect("test");
    assert!(hnsw.is_quantizer_trained());

    let results = hnsw.search_with_config(&vectors[query_id], k, 100, &int8_path_config());

    assert_top1_and_recall(&results, &vectors, query_id, DistanceMetric::Cosine, k);
    for (id, score) in &results {
        assert!(
            (0.0..=1.0).contains(score),
            "Cosine score for node {id} should be in [0,1], got {score}"
        );
    }
}

/// Cosine int8 traversal must survive an UNNORMALIZED query: the backend
/// normalizes before quantizing (codes are built from stored unit-norm
/// vectors), so scaling the query must not change the result set.
#[test]
fn test_int8_cosine_unnormalized_query_matches_normalized() {
    let (dim, n, k) = (32, 100, 10);
    let query_id = 42_usize;
    let engine = CachedSimdDistance::new(DistanceMetric::Cosine, dim);
    let hnsw = Sq8PrecisionHnsw::new(engine, dim, 16, 200, 1000).expect("test");

    let vectors = planted_unit_vectors(n, dim, query_id);
    for v in &vectors {
        hnsw.insert(v).expect("test");
    }
    hnsw.force_train_quantizer().expect("test");

    let scaled: Vec<f32> = vectors[query_id].iter().map(|x| x * 37.5).collect();
    let from_unit = hnsw.search_with_config(&vectors[query_id], k, 100, &int8_path_config());
    let from_scaled = hnsw.search_with_config(&scaled, k, 100, &int8_path_config());

    let unit_ids: Vec<usize> = from_unit.iter().map(|&(id, _)| id).collect();
    let scaled_ids: Vec<usize> = from_scaled.iter().map(|&(id, _)| id).collect();
    assert_eq!(
        unit_ids, scaled_ids,
        "cosine is scale-invariant: int8 traversal must not depend on query norm"
    );
}

// =========================================================================
// Concurrency: interior-mutability inserts racing search and training
// =========================================================================

/// Concurrent inserts across the training threshold plus racing searches
/// must neither deadlock nor desynchronize the positional store: after the
/// dust settles, every vector's self-query must return its own node id.
#[test]
fn test_concurrent_inserts_and_search_keep_store_aligned() {
    use std::sync::Arc;

    let (dim, per_thread, threads) = (16, 60, 4);
    let engine = CachedSimdDistance::new(DistanceMetric::Euclidean, dim);
    // training_sample_size = min(1000, 100) = 100 < total inserts (240),
    // so training fires mid-flight under contention.
    let hnsw = Arc::new(Sq8PrecisionHnsw::new(engine, dim, 16, 100, 100).expect("test"));

    let handles: Vec<_> = (0..threads)
        .map(|t| {
            let hnsw = Arc::clone(&hnsw);
            std::thread::spawn(move || {
                for i in 0..per_thread {
                    let seed = t * per_thread + i;
                    let v: Vec<f32> = (0..dim)
                        .map(|j| ((seed * dim + j) as f32 * 0.017).sin())
                        .collect();
                    hnsw.insert(&v).expect("concurrent insert");
                    if i % 8 == 0 {
                        // Race the search path against inserts/training.
                        let _ = hnsw.search(&v, 3, 30);
                    }
                }
            })
        })
        .collect();
    for handle in handles {
        handle.join().expect("insert thread panicked");
    }

    assert_eq!(hnsw.len(), threads * per_thread);
    assert!(hnsw.is_quantizer_trained(), "threshold crossed → trained");

    // Store alignment: with the vectors read back from the graph itself,
    // every node's self-query through the int8 path must rank itself first.
    let config = int8_path_config();
    for node_id in [0_usize, 57, 119, 200, 239] {
        let vector = hnsw
            .inner
            .with_vectors_read(|vectors| vectors.get(node_id).map(<[f32]>::to_vec))
            .expect("node vector present");
        let results = hnsw.search_with_config(&vector, 1, 60, &config);
        assert_eq!(
            results.first().map(|&(id, _)| id),
            Some(node_id),
            "self-query must rank node {node_id} first — a shifted store entry surfaces here"
        );
    }
}

// =========================================================================
// Recall contract (wired search path)
// =========================================================================

/// Verifies recall@10 >= 0.95 on 10K vectors with the DEFAULT config — the
/// exact configuration the collection search path runs (auto-trained after
/// 1000 inserts, int8 traversal active at the default min_index_size).
#[test]
fn test_sq8_precision_recall_above_threshold() {
    let dim = 128;
    let n = 10_000;
    let k = 10;
    let ef_search = 200;

    let engine = CachedSimdDistance::new(DistanceMetric::Euclidean, dim);
    let hnsw = Sq8PrecisionHnsw::new(engine, dim, 32, 200, n).expect("test");

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
    assert!(hnsw.is_quantizer_trained(), "auto-trained at 1000 inserts");

    let query_indices = [0, 1000, 5000, 7777, 9999];
    let mut total_recall = 0.0;

    for &qi in &query_indices {
        let query = &vectors[qi];

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

        // Default-config search: n == min_index_size → int8 path active.
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
        "SQ8 recall@{k} should be >= 0.95, got {avg_recall:.3}"
    );
}
