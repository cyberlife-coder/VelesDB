//! Tests for `RaBitQPrecisionHnsw`.
//!
//! The state-machine contracts shared with every quantized-precision codec
//! (lazy training, install alignment, rerank semantics, fallback guards,
//! recall) are pinned through the generic suites in
//! [`precision_test_support`](super::precision_test_support); this module
//! instantiates them for the `RaBitQ` codec and adds the `RaBitQ`-specific
//! pins (binary-path config contract, metric-ordered rerank on unit
//! vectors).

use super::precision_test_support as suite;
use super::rabitq_precision::{RaBitQCodec, RaBitQPrecisionConfig};
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
// Shared state-machine contracts, instantiated for the RaBitQ codec
// =========================================================================

#[test]
fn test_rabitq_precision_empty_index() {
    suite::check_empty_index::<RaBitQCodec>(64);
}

#[test]
fn test_rabitq_precision_fallback_when_untrained() {
    suite::check_untrained_fallback::<RaBitQCodec>(32);
}

#[test]
fn test_rabitq_precision_insert_trains_lazily() {
    suite::check_lazy_train_at_threshold::<RaBitQCodec>(64);
}

#[test]
fn test_rabitq_precision_force_train() {
    suite::check_force_train::<RaBitQCodec>(64);
}

#[test]
fn test_rabitq_precision_search_after_training() {
    suite::check_post_training_search_sorted::<RaBitQCodec>(64);
}

#[test]
fn test_rabitq_precision_insert_after_training() {
    suite::check_insert_after_training_alignment::<RaBitQCodec>(32);
}

#[test]
fn test_rabitq_euclidean_returns_sqrt_not_squared() {
    suite::check_euclidean_rerank_is_sqrt::<RaBitQCodec>(32);
}

/// Below `min_index_size`, a TRAINED index must skip the binary path and
/// return exactly the pre-training f32 results.
#[test]
fn test_rabitq_below_min_index_size_falls_back_to_f32() {
    suite::check_below_min_index_fallback::<RaBitQCodec>(32);
}

/// Verifies recall@10 >= 0.95 on 10K vectors with `RaBitQ` traversal
/// through the default configuration (EPIC-055).
#[test]
fn test_rabitq_precision_recall_above_threshold() {
    suite::check_recall_10k::<RaBitQCodec>();
}

// =========================================================================
// install_trained_rabitq (quantization wiring across restarts)
// =========================================================================

/// Trains a `RaBitQ` quantizer from samples (generic-suite adapter).
#[cfg(feature = "persistence")]
fn train_rabitq(samples: &[Vec<f32>]) -> std::sync::Arc<crate::quantization::RaBitQIndex> {
    std::sync::Arc::new(crate::quantization::RaBitQIndex::train(samples, 42).expect("train"))
}

/// Installing a pre-trained quantizer must encode every existing vector and
/// activate `RaBitQ` search with recall parity against the f32 baseline
/// (>= 0.7 — binary codes are coarser than SQ8's).
#[cfg(feature = "persistence")]
#[test]
fn test_install_trained_rabitq_encodes_existing_vectors() {
    suite::check_install_encodes_existing::<RaBitQCodec>(64, train_rabitq, 0.7);
}

/// Inserts after install must stay aligned with NodeId order.
#[cfg(feature = "persistence")]
#[test]
fn test_install_trained_rabitq_then_insert_keeps_alignment() {
    suite::check_install_then_insert_alignment::<RaBitQCodec>(64, train_rabitq);
}

// =========================================================================
// RaBitQ-specific pins
// =========================================================================

/// Default `min_index_size` must match the documented threshold (5000).
#[test]
fn test_rabitq_precision_config_default_min_index_size() {
    assert_eq!(RaBitQPrecisionConfig::default().min_index_size, 5000);
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
    let (dim, k) = (64, 10);
    let (hnsw, vectors) = suite::trained_planted_backend::<RaBitQCodec>(metric, dim);
    let query = &vectors[suite::PLANTED_QUERY_ID];

    let results = hnsw.search_with_config(query, k, 100, &binary_path_config());

    suite::assert_top1_and_recall(&results, &vectors, suite::PLANTED_QUERY_ID, metric, k);
}

#[test]
fn test_rabitq_cosine_rerank_keeps_best_candidates() {
    run_rabitq_self_query(DistanceMetric::Cosine);
}

#[test]
fn test_rabitq_dot_product_rerank_keeps_best_candidates() {
    run_rabitq_self_query(DistanceMetric::DotProduct);
}
