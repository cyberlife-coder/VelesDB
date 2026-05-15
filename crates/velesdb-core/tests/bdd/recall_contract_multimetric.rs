//! BDD-style recall contract tests — non-cosine metrics.
//!
//! The original `recall_contract.rs` exercises only the cosine metric, even
//! though VelesDB advertises five (Cosine, Euclidean, DotProduct, Hamming,
//! Jaccard). This file fills the recall coverage gap for the two metrics
//! that are most commonly chosen alongside cosine in real workloads —
//! Euclidean and DotProduct — by running the same 1K-vector / 64D dataset
//! shape through HNSW Balanced mode and comparing against brute-force
//! ground truth computed with the matching distance.
//!
//! Hamming and Jaccard operate on binary vectors and have a different
//! recall failure mode (data-distribution-driven); they are validated by
//! their dedicated property tests in `crates/velesdb-core/tests/`.
//!
//! # Threshold
//!
//! Balanced mode (≥ 0.95) is the single mode tested here. Fast/Accurate/
//! Perfect already have cross-metric proof via `vector_search.rs` BDD
//! scenarios. The point is to guarantee the *recall claim* — that
//! non-cosine metrics actually meet the 0.95 floor on a representative
//! dataset, not that the planner picks the right ef value.
//!
//! # Why this matters
//!
//! Without this file a regression in the Euclidean or DotProduct kernel
//! (a SIMD path bug, a wrong distance sign, a missing normalization)
//! would land on `develop` without tripping any recall test. The CTO
//! audit on 2026-05-15 surfaced this gap.

use std::collections::HashSet;

use velesdb_core::{Database, DistanceMetric, Point, SearchQuality};

const DIM: usize = 64;
const NUM_VECTORS: usize = 1_000;
const K: usize = 10;
const NUM_QUERIES: usize = 10;
const RECALL_FLOOR: f64 = 0.95;

// =========================================================================
// Helpers
// =========================================================================

/// Deterministic pseudo-random vector generator (xorshift64).
#[allow(clippy::cast_precision_loss)]
fn generate_vectors(count: usize, dim: usize, seed: u64) -> Vec<Vec<f32>> {
    let mut state = seed;
    (0..count)
        .map(|_| {
            (0..dim)
                .map(|_| {
                    state ^= state << 13;
                    state ^= state >> 7;
                    state ^= state << 17;
                    (state as f32 / u64::MAX as f32) * 2.0 - 1.0
                })
                .collect()
        })
        .collect()
}

fn euclidean_distance(a: &[f32], b: &[f32]) -> f32 {
    a.iter()
        .zip(b.iter())
        .map(|(x, y)| (x - y) * (x - y))
        .sum::<f32>()
        .sqrt()
}

fn dot_product(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

/// Brute-force k-NN with metric-specific scoring + ordering.
fn brute_force_knn(
    vectors: &[Vec<f32>],
    query: &[f32],
    k: usize,
    metric: DistanceMetric,
) -> Vec<u64> {
    let mut scored: Vec<(u64, f32)> = vectors
        .iter()
        .enumerate()
        .map(|(i, v)| {
            #[allow(clippy::cast_possible_truncation)]
            let id = i as u64;
            let score = match metric {
                DistanceMetric::Euclidean => euclidean_distance(query, v),
                // DotProduct: higher = closer, so negate to make it a distance.
                DistanceMetric::DotProduct => -dot_product(query, v),
                _ => unreachable!("test covers only Euclidean and DotProduct"),
            };
            (id, score)
        })
        .collect();
    scored.sort_by(|a, b| {
        a.1.partial_cmp(&b.1)
            .expect("test: no NaN in synthetic vectors")
    });
    scored.truncate(k);
    scored.iter().map(|(id, _)| *id).collect()
}

#[allow(clippy::cast_precision_loss)]
fn compute_recall(retrieved: &[u64], ground_truth: &[u64], k: usize) -> f64 {
    let k = k.min(retrieved.len()).min(ground_truth.len());
    if k == 0 {
        return 0.0;
    }
    let retrieved_set: HashSet<_> = retrieved.iter().take(k).collect();
    let gt_set: HashSet<_> = ground_truth.iter().take(k).collect();
    retrieved_set.intersection(&gt_set).count() as f64 / k as f64
}

#[allow(clippy::cast_precision_loss)]
fn measure_balanced_recall(metric: DistanceMetric) -> f64 {
    let vectors = generate_vectors(NUM_VECTORS, DIM, 42);
    let queries = generate_vectors(NUM_QUERIES, DIM, 456);

    let dir = tempfile::TempDir::new().expect("test: temp dir");
    let db = Database::open(dir.path()).expect("test: open database");
    db.create_vector_collection("recall_test", DIM, metric)
        .expect("test: create collection");
    let vc = db
        .get_vector_collection("recall_test")
        .expect("test: get collection");

    #[allow(clippy::cast_possible_truncation)]
    let points: Vec<Point> = vectors
        .iter()
        .enumerate()
        .map(|(i, v)| Point::new(i as u64, v.clone(), None))
        .collect();
    vc.upsert(points).expect("test: upsert vectors");

    let total: f64 = queries
        .iter()
        .map(|q| {
            let retrieved = vc
                .search_with_quality(q, K, SearchQuality::Balanced)
                .expect("test: search")
                .iter()
                .map(|r| r.point.id)
                .collect::<Vec<_>>();
            let ground_truth = brute_force_knn(&vectors, q, K, metric);
            compute_recall(&retrieved, &ground_truth, K)
        })
        .sum();
    total / queries.len() as f64
}

// =========================================================================
// BDD Scenarios
// =========================================================================

#[test]
fn test_recall_contract_balanced_euclidean_gte_95() {
    // GIVEN a Euclidean collection with 1K vectors (64D)
    // WHEN  searching 10 queries in Balanced mode
    // THEN  average recall@10 >= 0.95
    let avg_recall = measure_balanced_recall(DistanceMetric::Euclidean);
    assert!(
        avg_recall >= RECALL_FLOOR,
        "Euclidean Balanced recall@{K} should be >= {RECALL_FLOOR}, got {avg_recall:.3}"
    );
}

#[test]
fn test_recall_contract_balanced_dot_product_gte_95() {
    // GIVEN a DotProduct collection with 1K vectors (64D)
    // WHEN  searching 10 queries in Balanced mode
    // THEN  average recall@10 >= 0.95
    let avg_recall = measure_balanced_recall(DistanceMetric::DotProduct);
    assert!(
        avg_recall >= RECALL_FLOOR,
        "DotProduct Balanced recall@{K} should be >= {RECALL_FLOOR}, got {avg_recall:.3}"
    );
}
