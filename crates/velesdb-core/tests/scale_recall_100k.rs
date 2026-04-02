//! Recall quality gate at 100K scale.
//!
//! Validates that HNSW recall does not degrade at production-relevant scale
//! (100K vectors, 128 dimensions, cosine metric).
//!
//! # Running
//!
//! ```bash
//! cargo test -p velesdb-core --test scale_recall_100k \
//!     --features persistence -- --ignored --nocapture --test-threads=1
//! ```

use velesdb_core::index::hnsw::{HnswParams, SearchQuality};
use velesdb_core::index::HnswIndex;
use velesdb_core::metrics::recall_at_k;
use velesdb_core::DistanceMetric;

const NUM_VECTORS: usize = 100_000;
const DIMENSION: usize = 128;
const NUM_QUERIES: usize = 100;
const K: usize = 10;

/// Generates a deterministic, normalized vector from a seed.
#[allow(clippy::cast_precision_loss)]
fn generate_vector(dim: usize, seed: u64) -> Vec<f32> {
    let mut v = Vec::with_capacity(dim);
    let mut s = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
    for _ in 0..dim {
        s = s.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        v.push(((s >> 33) as f32) / (u32::MAX as f32) - 0.5);
    }
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in &mut v {
            *x /= norm;
        }
    }
    v
}

/// Brute-force cosine nearest neighbors (ground truth).
#[allow(clippy::cast_precision_loss)]
fn brute_force_cosine_knn(vectors: &[Vec<f32>], query: &[f32], k: usize) -> Vec<u64> {
    let mut dists: Vec<(u64, f32)> = vectors
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let dot: f32 = v.iter().zip(query.iter()).map(|(a, b)| a * b).sum();
            let dist = 1.0 - dot; // cosine distance (vectors are normalized)
            (i as u64, dist)
        })
        .collect();

    dists.sort_by(|a, b| a.1.partial_cmp(&b.1).expect("test: no NaN"));
    dists.iter().take(k).map(|(id, _)| *id).collect()
}

/// Measures mean recall over multiple queries at a given quality level.
#[allow(clippy::cast_precision_loss)]
fn measure_recall(
    index: &HnswIndex,
    vectors: &[Vec<f32>],
    queries: &[Vec<f32>],
    quality: SearchQuality,
) -> f64 {
    let mut total = 0.0;
    for query in queries {
        let results: Vec<u64> = index
            .search_with_quality(query, K, quality)
            .unwrap()
            .iter()
            .map(|r| r.id)
            .collect();
        let ground_truth = brute_force_cosine_knn(vectors, query, K);
        total += recall_at_k(&ground_truth, &results);
    }
    total / queries.len() as f64
}

/// Builds the 100K index and query set, shared across tests.
fn build_100k_fixture() -> (HnswIndex, Vec<Vec<f32>>, Vec<Vec<f32>>) {
    let vectors: Vec<Vec<f32>> = (0..NUM_VECTORS)
        .map(|i| generate_vector(DIMENSION, i as u64))
        .collect();

    let params = HnswParams::for_dataset_size(DIMENSION, NUM_VECTORS);
    let index = HnswIndex::with_params(DIMENSION, DistanceMetric::Cosine, params)
        .expect("test: index creation");

    let batch: Vec<(u64, &[f32])> = vectors
        .iter()
        .enumerate()
        .map(|(i, v)| (i as u64, v.as_slice()))
        .collect();
    let inserted = index.insert_batch_parallel(batch);
    assert_eq!(inserted, NUM_VECTORS, "test: all vectors inserted");

    let queries: Vec<Vec<f32>> = (0..NUM_QUERIES)
        .map(|i| generate_vector(DIMENSION, (NUM_VECTORS + i) as u64))
        .collect();

    (index, vectors, queries)
}

#[test]
#[ignore = "Long-running 100K recall gate — run with --ignored"]
fn scale_100k_recall_balanced() {
    let (index, vectors, queries) = build_100k_fixture();
    let recall = measure_recall(&index, &vectors, &queries, SearchQuality::Balanced);

    println!();
    println!("=== 100K Recall Gate: Balanced ===");
    println!("  recall@{K} = {recall:.4} (threshold: 0.95)");

    assert!(
        recall >= 0.95,
        "Balanced recall@{K} = {recall:.4} is below 0.95 threshold at 100K scale"
    );
}

#[test]
#[ignore = "Long-running 100K recall gate — run with --ignored"]
fn scale_100k_recall_fast() {
    let (index, vectors, queries) = build_100k_fixture();
    let recall = measure_recall(&index, &vectors, &queries, SearchQuality::Fast);

    println!();
    println!("=== 100K Recall Gate: Fast ===");
    println!("  recall@{K} = {recall:.4} (threshold: 0.90)");

    assert!(
        recall >= 0.90,
        "Fast recall@{K} = {recall:.4} is below 0.90 threshold at 100K scale"
    );
}

#[test]
#[ignore = "Long-running 100K recall gate — run with --ignored"]
fn scale_100k_recall_accurate() {
    let (index, vectors, queries) = build_100k_fixture();
    let recall = measure_recall(&index, &vectors, &queries, SearchQuality::Accurate);

    println!();
    println!("=== 100K Recall Gate: Accurate ===");
    println!("  recall@{K} = {recall:.4} (threshold: 0.98)");

    assert!(
        recall >= 0.98,
        "Accurate recall@{K} = {recall:.4} is below 0.98 threshold at 100K scale"
    );
}

#[test]
#[ignore = "Long-running 100K recall gate — run with --ignored"]
fn scale_100k_recall_perfect() {
    let (index, vectors, queries) = build_100k_fixture();
    let recall = measure_recall(&index, &vectors, &queries, SearchQuality::Perfect);

    println!();
    println!("=== 100K Recall Gate: Perfect ===");
    println!("  recall@{K} = {recall:.4} (threshold: 1.00)");

    assert!(
        (recall - 1.0).abs() < f64::EPSILON,
        "Perfect recall@{K} = {recall:.4} must be 1.00 at any scale"
    );
}
