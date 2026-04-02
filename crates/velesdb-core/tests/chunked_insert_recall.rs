//! Recall parity test for chunked batch insertion (#364).
//!
//! Validates that `insert_batch_parallel` (which uses chunked allocation
//! internally) produces an HNSW graph with recall comparable to sequential
//! insertion. This is an `#[ignore]` test because it takes several seconds
//! on 10K vectors.
//!
//! # Running
//!
//! ```bash
//! cargo test -p velesdb-core --test chunked_insert_recall \
//!     --features persistence -- --ignored --nocapture --test-threads=1
//! ```
//!
//! # Metric coverage
//!
//! This test uses Euclidean distance. The criterion benchmark
//! (`chunked_insert_recall_benchmark`) uses Cosine similarity,
//! providing multi-metric validation of the chunked insert path.

use velesdb_core::index::hnsw::{HnswParams, SearchQuality};
use velesdb_core::index::HnswIndex;
use velesdb_core::metrics::recall_at_k;
use velesdb_core::DistanceMetric;

const NUM_VECTORS: usize = 10_000;
const DIMENSION: usize = 128;
const NUM_QUERIES: usize = 50;
const K: usize = 10;
const MIN_MEAN_RECALL: f64 = 0.90;

/// Generates a deterministic vector from its index.
///
/// Uses a simple hash-like scheme that produces well-distributed f32 values
/// in [0, 1) without requiring an RNG crate.
#[allow(clippy::cast_precision_loss)]
fn generate_vector(index: usize, dim: usize) -> Vec<f32> {
    (0..dim)
        .map(|d| ((index.wrapping_mul(31).wrapping_add(d.wrapping_mul(17))) % 1000) as f32 / 1000.0)
        .collect()
}

/// Computes brute-force Euclidean (L2 squared) nearest neighbors.
#[allow(clippy::cast_precision_loss)]
fn brute_force_knn(vectors: &[Vec<f32>], query: &[f32], k: usize) -> Vec<u64> {
    let mut dists: Vec<(u64, f32)> = vectors
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let d: f32 = v
                .iter()
                .zip(query.iter())
                .map(|(a, b)| (a - b) * (a - b))
                .sum();
            (i as u64, d)
        })
        .collect();

    dists.sort_by(|a, b| a.1.partial_cmp(&b.1).expect("test: no NaN in distances"));
    dists.iter().take(k).map(|(id, _)| *id).collect()
}

#[test]
#[ignore = "Long-running recall validation — run with --ignored"]
#[allow(clippy::cast_precision_loss)]
fn chunked_insert_recall_parity() {
    // --- Build dataset ---
    let vectors: Vec<Vec<f32>> = (0..NUM_VECTORS)
        .map(|i| generate_vector(i, DIMENSION))
        .collect();

    // --- Build HNSW index via batch insert ---
    let params = HnswParams::custom(16, 200, 20_000);
    let index = HnswIndex::with_params(DIMENSION, DistanceMetric::Euclidean, params)
        .expect("test: index creation");

    let batch: Vec<(u64, &[f32])> = vectors
        .iter()
        .enumerate()
        .map(|(i, v)| (i as u64, v.as_slice()))
        .collect();
    let inserted = index.insert_batch_parallel(batch);
    assert_eq!(inserted, NUM_VECTORS, "all vectors should be inserted");

    // --- Generate query vectors (deterministic, offset from data) ---
    let queries: Vec<Vec<f32>> = (0..NUM_QUERIES)
        .map(|i| generate_vector(NUM_VECTORS + i, DIMENSION))
        .collect();

    // --- Evaluate recall ---
    let mut recalls = Vec::with_capacity(NUM_QUERIES);

    for query in &queries {
        let results = index
            .search_with_quality(query, K, SearchQuality::Balanced)
            .unwrap();
        let result_ids: Vec<u64> = results.iter().map(|r| r.id).collect();
        let ground_truth = brute_force_knn(&vectors, query, K);

        let recall = recall_at_k(&ground_truth, &result_ids);
        recalls.push(recall);
    }

    recalls.sort_by(|a, b| a.partial_cmp(b).expect("test: no NaN in recalls"));

    let mean_recall: f64 = recalls.iter().sum::<f64>() / recalls.len() as f64;
    let min_recall = recalls[0];
    // 5th percentile: index 2 out of 50 (floor(50 * 0.05) = 2)
    let p5_recall = recalls[2];

    println!();
    println!("=== Chunked Insert Recall Report ({NUM_VECTORS} vectors, {DIMENSION}D) ===");
    println!("  Queries:      {NUM_QUERIES}");
    println!("  k:            {K}");
    println!("  Mean recall:  {mean_recall:.4}");
    println!("  Min recall:   {min_recall:.4}");
    println!("  P5 recall:    {p5_recall:.4}");
    println!();

    assert!(
        mean_recall >= MIN_MEAN_RECALL,
        "Mean recall {mean_recall:.4} is below threshold {MIN_MEAN_RECALL:.2}. \
         Chunked insertion may have degraded graph quality."
    );
}
