//! Tests for `Sq8PrecisionHnsw` (int8 traversal + f32 re-ranking).
//!
//! The state-machine contracts shared with every quantized-precision codec
//! are pinned through the generic suites in
//! [`precision_test_support`](super::precision_test_support); this module
//! instantiates them for the SQ8 codec and adds the SQ8-specific pins:
//! metric gating, cosine query normalization, and store alignment under
//! concurrent inserts.

use super::distance::CachedSimdDistance;
use super::precision_test_support as suite;
use super::quantization::ScalarQuantizer;
use super::sq8_precision::{Sq8Codec, Sq8PrecisionConfig, Sq8PrecisionHnsw};
use crate::distance::DistanceMetric;
use std::sync::Arc;

/// Config that forces the int8 path on small test indexes (the default
/// `min_index_size` of 10 000 would route them to the exact-f32 fallback).
fn int8_path_config() -> Sq8PrecisionConfig {
    Sq8PrecisionConfig {
        min_index_size: 0,
        ..Sq8PrecisionConfig::default()
    }
}

// =========================================================================
// Shared state-machine contracts, instantiated for the SQ8 codec
// =========================================================================

#[test]
fn test_sq8_precision_empty_index() {
    suite::check_empty_index::<Sq8Codec>(128);
}

#[test]
fn test_sq8_precision_fallback_when_untrained() {
    suite::check_untrained_fallback::<Sq8Codec>(32);
}

#[test]
fn test_sq8_precision_insert_trains_lazily() {
    suite::check_lazy_train_at_threshold::<Sq8Codec>(32);
}

#[test]
fn test_sq8_precision_force_train() {
    suite::check_force_train::<Sq8Codec>(32);
}

#[test]
fn test_sq8_precision_search_after_training() {
    suite::check_post_training_search_sorted::<Sq8Codec>(32);
}

#[test]
fn test_sq8_precision_insert_after_training() {
    suite::check_insert_after_training_alignment::<Sq8Codec>(32);
}

#[test]
fn test_sq8_euclidean_returns_sqrt_not_squared() {
    suite::check_euclidean_rerank_is_sqrt::<Sq8Codec>(32);
}

/// Below `min_index_size`, a TRAINED index must skip the int8 path and
/// return exactly the pre-training f32 results.
#[test]
fn test_sq8_below_min_index_size_falls_back_to_f32() {
    suite::check_below_min_index_fallback::<Sq8Codec>(32);
}

/// Verifies recall@10 >= 0.95 on 10K vectors with int8 traversal through
/// the default configuration — the exact shape the collection path runs.
#[test]
fn test_sq8_precision_recall_above_threshold() {
    suite::check_recall_10k::<Sq8Codec>();
}

// =========================================================================
// install_trained_sq8 (persistence wiring across restarts)
// =========================================================================

/// Trains an SQ8 quantizer from samples (generic-suite adapter).
fn train_sq8(samples: &[Vec<f32>]) -> Arc<ScalarQuantizer> {
    let refs: Vec<&[f32]> = samples.iter().map(Vec::as_slice).collect();
    Arc::new(ScalarQuantizer::train(&refs).expect("train"))
}

/// Installing a pre-trained quantizer must encode every existing vector and
/// activate int8 search with recall parity against the f32 baseline
/// (>= 0.9 — int8 codes are finer than `RaBitQ` bits).
#[test]
fn test_install_trained_sq8_encodes_existing_vectors() {
    suite::check_install_encodes_existing::<Sq8Codec>(64, train_sq8, 0.9);
}

/// Inserts after install must stay aligned with NodeId order.
#[test]
fn test_install_trained_sq8_then_insert_keeps_alignment() {
    suite::check_install_then_insert_alignment::<Sq8Codec>(64, train_sq8);
}

// =========================================================================
// SQ8-specific pins: config contract
// =========================================================================

/// Defaults must match the documented contract, and with a small index the
/// default config must take the f32 fallback (bit-identical to `search`).
#[test]
fn test_sq8_precision_config_defaults() {
    let config = Sq8PrecisionConfig::default();
    assert_eq!(config.oversampling_ratio, 4);
    assert_eq!(config.min_index_size, 10_000);

    let engine = CachedSimdDistance::new_prenormalized(DistanceMetric::Euclidean, 32);
    let hnsw = Sq8PrecisionHnsw::new(engine, 32, 16, 100, 1000).expect("test");
    for i in 0..50 {
        let v: Vec<f32> = (0..32).map(|j| (i * 32 + j) as f32).collect();
        hnsw.insert(&v).expect("test");
    }
    hnsw.force_train_quantizer().expect("test");
    let query: Vec<f32> = (0..32).map(|j| j as f32).collect();
    let with_default = hnsw.search_with_config(&query, 10, 50, &config);
    let plain = hnsw.search(&query, 10, 50);
    assert_eq!(
        with_default, plain,
        "default min_index_size=10_000 must force small indexes onto the f32 fallback path"
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
    let engine = CachedSimdDistance::new_prenormalized(DistanceMetric::DotProduct, 32);
    let hnsw = Sq8PrecisionHnsw::new(engine, 32, 16, 100, 100).expect("test");

    // Past the lazy-train threshold — a supported metric would have trained.
    for v in &suite::sinusoidal_vectors(100, 32) {
        hnsw.insert(v).expect("test");
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
    let engine = CachedSimdDistance::new_prenormalized(DistanceMetric::Hamming, 8);
    let hnsw = Sq8PrecisionHnsw::new(engine, 8, 16, 100, 1000).expect("test");
    for i in 0..10 {
        let v: Vec<f32> = (0..8).map(|j| ((i + j) % 2) as f32).collect();
        hnsw.insert(&v).expect("test");
    }

    let samples: Vec<Vec<f32>> = vec![vec![0.0; 8], vec![1.0; 8]];
    let installed = hnsw
        .install_trained_sq8(train_sq8(&samples))
        .expect("install call");
    assert!(!installed, "install must be refused on Hamming");
    assert!(!hnsw.is_quantizer_trained());
}

// =========================================================================
// Cosine through the int8 path: normalized query + code space
// =========================================================================

/// The query and the codes must both live in the normalized (stored) vector
/// space, the rerank must sort by similarity (higher = better), and scores
/// must be clamped to [0, 1].
#[test]
fn test_int8_cosine_rerank_keeps_best_candidates() {
    let (dim, k) = (32, 10);
    let (hnsw, vectors) = suite::trained_planted_backend::<Sq8Codec>(DistanceMetric::Cosine, dim);
    let query = &vectors[suite::PLANTED_QUERY_ID];

    let results = hnsw.search_with_config(query, k, 100, &int8_path_config());

    suite::assert_top1_and_recall(
        &results,
        &vectors,
        suite::PLANTED_QUERY_ID,
        DistanceMetric::Cosine,
        k,
    );
    for (id, score) in &results {
        assert!(
            (0.0..=1.0).contains(score),
            "Cosine score for node {id} should be in [0,1], got {score}"
        );
    }
}

/// Cosine int8 traversal must survive an unnormalized query.
#[test]
fn test_int8_cosine_unnormalized_query_matches_normalized() {
    suite::check_cosine_scale_invariance::<Sq8Codec>(32);
}

// =========================================================================
// Concurrency: interior-mutability inserts racing search and training
// =========================================================================

/// Concurrent inserts across the training threshold plus racing searches
/// must neither deadlock nor desynchronize the positional store: after the
/// dust settles, every vector's self-query must return its own node id.
#[test]
fn test_concurrent_inserts_and_search_keep_store_aligned() {
    let (dim, per_thread, threads) = (16, 60, 4);
    let engine = CachedSimdDistance::new_prenormalized(DistanceMetric::Euclidean, dim);
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
