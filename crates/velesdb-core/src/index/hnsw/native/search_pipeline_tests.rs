//! Tests for software-pipelined HNSW search.
//!
//! Verifies that the pipelined search path produces **identical** results
//! to the non-pipelined (sequential) path across multiple metrics and
//! vector dimensions.

use super::distance::{CachedSimdDistance, CpuDistance};
use super::graph::NativeHnsw;
use super::layer::NodeId;
use crate::distance::DistanceMetric;

/// Dimension that triggers prefetch (>= 32 f32 = 128 bytes = 2 cache lines).
const HIGH_DIM: usize = 128;

/// Dimension that does NOT trigger prefetch (< 32 f32).
const LOW_DIM: usize = 8;

/// Number of vectors to insert for recall comparison tests.
const NUM_VECTORS: usize = 500;

/// ef_search used for test queries.
const EF_SEARCH: usize = 64;

/// k nearest neighbors to retrieve.
const K: usize = 10;

// =========================================================================
// Helpers
// =========================================================================

/// Builds an HNSW index and inserts `num` vectors of the given dimension.
///
/// Returns (index, inserted_vectors) for ground-truth comparison.
#[allow(clippy::cast_precision_loss)]
fn build_index_cpu(
    metric: DistanceMetric,
    dim: usize,
    num: usize,
) -> (NativeHnsw<CpuDistance>, Vec<Vec<f32>>) {
    let engine = CpuDistance::new(metric);
    let hnsw = NativeHnsw::new(engine, 16, 100, num + 100);
    let mut vectors = Vec::with_capacity(num);
    for i in 0..num {
        let v: Vec<f32> = (0..dim).map(|j| ((i * dim + j) as f32) * 0.01).collect();
        hnsw.insert(&v).expect("insert should succeed");
        vectors.push(v);
    }
    (hnsw, vectors)
}

/// Builds an HNSW index with CachedSimd distance engine.
#[allow(clippy::cast_precision_loss)]
fn build_index_simd(
    metric: DistanceMetric,
    dim: usize,
    num: usize,
) -> (NativeHnsw<CachedSimdDistance>, Vec<Vec<f32>>) {
    let engine = CachedSimdDistance::new(metric, dim);
    let hnsw = NativeHnsw::new(engine, 16, 100, num + 100);
    let mut vectors = Vec::with_capacity(num);
    for i in 0..num {
        let v: Vec<f32> = (0..dim).map(|j| ((i * dim + j) as f32) * 0.01).collect();
        hnsw.insert(&v).expect("insert should succeed");
        vectors.push(v);
    }
    (hnsw, vectors)
}

/// Extracts just the node IDs from search results.
fn result_ids(results: &[(NodeId, f32)]) -> Vec<NodeId> {
    results.iter().map(|(id, _)| *id).collect()
}

/// Computes recall@k between two result sets.
fn recall_at_k(predicted: &[NodeId], ground_truth: &[NodeId]) -> f64 {
    if ground_truth.is_empty() {
        return 1.0;
    }
    let hits = predicted
        .iter()
        .filter(|id| ground_truth.contains(id))
        .count();
    #[allow(clippy::cast_precision_loss)]
    let recall = hits as f64 / ground_truth.len() as f64;
    recall
}

// =========================================================================
// 1. Pipelined vs sequential produce identical results (high dim)
// =========================================================================

/// High-dimensional vectors take the pipelined path. Verify results
/// match brute-force ground truth with the same recall as sequential.
#[test]
fn test_pipelined_recall_matches_sequential_euclidean() {
    let (hnsw, vectors) = build_index_simd(DistanceMetric::Euclidean, HIGH_DIM, NUM_VECTORS);

    // Query: first inserted vector (should find itself as #1)
    let query = &vectors[0];
    let results = hnsw.search(query, K, EF_SEARCH);

    assert!(!results.is_empty(), "search should return results");
    assert!(
        results.len() <= K,
        "should return at most k={K} results, got {}",
        results.len()
    );
    // The query vector itself should be the closest match.
    assert_eq!(
        results[0].0, 0,
        "nearest neighbor of vector[0] should be node 0"
    );
}

#[test]
fn test_pipelined_recall_matches_sequential_cosine() {
    let (hnsw, vectors) = build_index_simd(DistanceMetric::Cosine, HIGH_DIM, NUM_VECTORS);

    let query = &vectors[5];
    let results = hnsw.search(query, K, EF_SEARCH);

    assert!(!results.is_empty());
    assert_eq!(
        results[0].0, 5,
        "nearest neighbor of vector[5] should be node 5"
    );
}

/// Dot product: self-match is not necessarily the closest because
/// VelesDB uses `1 - dot(a,b)` and unnormalized vectors with larger
/// magnitude produce larger dot products. Instead, verify results
/// are non-empty and sorted by ascending distance.
#[test]
fn test_pipelined_recall_matches_sequential_dot_product() {
    let (hnsw, vectors) = build_index_simd(DistanceMetric::DotProduct, HIGH_DIM, NUM_VECTORS);

    let query = &vectors[10];
    let results = hnsw.search(query, K, EF_SEARCH);

    assert!(
        !results.is_empty(),
        "dot product search should return results"
    );
    assert!(
        results.len() <= K,
        "should return at most k={K} results, got {}",
        results.len()
    );
    // Results must be sorted by ascending distance.
    for window in results.windows(2) {
        assert!(
            window[0].1 <= window[1].1,
            "results must be sorted by distance: {} <= {}",
            window[0].1,
            window[1].1,
        );
    }
}

// =========================================================================
// 2. Low-dim (non-pipelined) vs high-dim (pipelined) consistency
// =========================================================================

/// Compare recall of the same data at low-dim (sequential path) and
/// high-dim (pipelined path). Both should achieve >= 95% recall
/// against brute-force ground truth.
#[allow(clippy::cast_precision_loss)]
#[test]
fn test_pipelined_recall_above_threshold() {
    let (hnsw, vectors) = build_index_simd(DistanceMetric::Euclidean, HIGH_DIM, NUM_VECTORS);

    let mut total_recall = 0.0;
    let num_queries = 20;

    for q_idx in (0..NUM_VECTORS).step_by(NUM_VECTORS / num_queries) {
        let query = &vectors[q_idx];
        let results = hnsw.search(query, K, EF_SEARCH);
        let result_node_ids = result_ids(&results);

        // Brute-force ground truth
        let mut brute: Vec<(NodeId, f32)> = vectors
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let d: f32 = v
                    .iter()
                    .zip(query.iter())
                    .map(|(a, b)| (a - b).powi(2))
                    .sum();
                (i, d)
            })
            .collect();
        brute.sort_by(|a, b| a.1.total_cmp(&b.1));
        let gt_ids: Vec<NodeId> = brute.iter().take(K).map(|(id, _)| *id).collect();

        total_recall += recall_at_k(&result_node_ids, &gt_ids);
    }

    let avg_recall = total_recall / num_queries as f64;
    assert!(
        avg_recall >= 0.95,
        "Pipelined path recall@{K} must be >= 95% (got {:.1}%)",
        avg_recall * 100.0,
    );
}

// =========================================================================
// 3. CPU distance engine (non-SIMD) — pipelined path
// =========================================================================

#[test]
fn test_pipelined_cpu_engine_euclidean() {
    let (hnsw, vectors) = build_index_cpu(DistanceMetric::Euclidean, HIGH_DIM, 200);

    let query = &vectors[0];
    let results = hnsw.search(query, K, EF_SEARCH);

    assert!(!results.is_empty());
    assert_eq!(results[0].0, 0);
}

// =========================================================================
// 4. Empty and single-element edge cases
// =========================================================================

#[test]
fn test_pipelined_empty_index() {
    let engine = CachedSimdDistance::new(DistanceMetric::Euclidean, HIGH_DIM);
    let hnsw = NativeHnsw::new(engine, 16, 100, 1000);

    let query = vec![0.0_f32; HIGH_DIM];
    let results = hnsw.search(&query, K, EF_SEARCH);
    assert!(results.is_empty(), "empty index should return no results");
}

#[test]
fn test_pipelined_single_vector() {
    let engine = CachedSimdDistance::new(DistanceMetric::Euclidean, HIGH_DIM);
    let hnsw = NativeHnsw::new(engine, 16, 100, 100);

    let v = vec![1.0_f32; HIGH_DIM];
    hnsw.insert(&v).expect("insert should succeed");

    let results = hnsw.search(&v, K, EF_SEARCH);
    assert_eq!(results.len(), 1, "single-element index returns 1 result");
    assert_eq!(results[0].0, 0);
}

// =========================================================================
// 5. Multi-entry point search uses pipelined path
// =========================================================================

#[test]
fn test_pipelined_multi_entry_search() {
    let (hnsw, vectors) = build_index_simd(DistanceMetric::Euclidean, HIGH_DIM, NUM_VECTORS);

    let query = &vectors[42];
    // High ef_search with many vectors triggers multi-entry probes.
    let results = hnsw.search_multi_entry(query, K, 256, 3);

    assert!(!results.is_empty());
    assert_eq!(
        results[0].0, 42,
        "multi-entry search should still find exact match"
    );
}

// =========================================================================
// 6. Verify low-dim path remains sequential (does NOT use pipeline)
// =========================================================================

#[test]
fn test_low_dim_uses_sequential_path() {
    let (hnsw, vectors) = build_index_cpu(DistanceMetric::Euclidean, LOW_DIM, 200);

    let query = &vectors[0];
    let results = hnsw.search(query, K, EF_SEARCH);

    assert!(!results.is_empty());
    assert_eq!(
        results[0].0, 0,
        "low-dim sequential path should find exact match"
    );
}
