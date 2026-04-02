//! BDD-style recall contract tests.
//!
//! These tests enforce the recall quality guarantees for each `SearchQuality`
//! mode on a 1K-vector synthetic dataset. They verify the user's perspective:
//! "when I search with mode X, recall@10 meets the documented threshold."
//!
//! # Dataset
//!
//! 1000 random vectors in 64 dimensions with cosine metric. Ground truth is
//! computed via brute-force search. Each query is repeated across 10 random
//! query vectors and the average recall is compared to the contract threshold.
//!
//! # Thresholds
//!
//! | Mode | Recall@10 |
//! |----------|-----------|
//! | Fast | >= 0.90 |
//! | Balanced | >= 0.95 |
//! | Accurate | >= 0.99 |
//! | Perfect | = 1.00 |

use std::collections::HashSet;

use velesdb_core::{Database, Point};

const DIM: usize = 64;
const NUM_VECTORS: usize = 1_000;
const K: usize = 10;
const NUM_QUERIES: usize = 10;

// =========================================================================
// Helpers
// =========================================================================

/// Deterministic pseudo-random vector generator (no external crate needed).
#[allow(clippy::cast_precision_loss)]
fn generate_vectors(count: usize, dim: usize, seed: u64) -> Vec<Vec<f32>> {
    let mut state = seed;
    (0..count)
        .map(|_| {
            (0..dim)
                .map(|_| {
                    // xorshift64
                    state ^= state << 13;
                    state ^= state >> 7;
                    state ^= state << 17;
                    (state as f32 / u64::MAX as f32) * 2.0 - 1.0
                })
                .collect()
        })
        .collect()
}

/// Brute-force cosine nearest neighbors (ground truth).
fn brute_force_cosine_knn(vectors: &[Vec<f32>], query: &[f32], k: usize) -> Vec<u64> {
    let mut scored: Vec<(u64, f32)> = vectors
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let sim = cosine_similarity(query, v);
            // Lower distance = better, so 1.0 - sim
            #[allow(clippy::cast_possible_truncation)]
            (i as u64, 1.0 - sim)
        })
        .collect();

    scored.sort_by(|a, b| a.1.partial_cmp(&b.1).expect("test: no NaN in distances"));
    scored.truncate(k);
    scored.iter().map(|(id, _)| *id).collect()
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a > 0.0 && norm_b > 0.0 {
        dot / (norm_a * norm_b)
    } else {
        0.0
    }
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

/// Create a database with a cosine collection containing the given vectors.
fn setup_recall_db(dir: &tempfile::TempDir, vectors: &[Vec<f32>]) -> Database {
    let db = Database::open(dir.path()).expect("test: open database");

    // Use VelesQL to create the collection
    let create_sql =
        format!("CREATE COLLECTION recall_test (dimension = {DIM}, metric = 'cosine');");
    let query = velesdb_core::velesql::Parser::parse(&create_sql).expect("test: parse CREATE");
    db.execute_query(&query, &std::collections::HashMap::new())
        .expect("test: execute CREATE");

    // Insert vectors via collection API
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
    db
}

/// Search using a specific `SearchQuality` mode and return result IDs.
fn search_with_quality(
    db: &Database,
    query: &[f32],
    k: usize,
    quality: velesdb_core::SearchQuality,
) -> Vec<u64> {
    let vc = db
        .get_vector_collection("recall_test")
        .expect("test: get collection");

    let results = vc
        .search_with_quality(query, k, quality)
        .expect("test: search");
    results.iter().map(|r| r.point.id).collect()
}

/// Run recall measurement across multiple queries and return average recall.
#[allow(clippy::cast_precision_loss)]
fn measure_avg_recall(
    db: &Database,
    query_vectors: &[Vec<f32>],
    all_vectors: &[Vec<f32>],
    quality: velesdb_core::SearchQuality,
) -> f64 {
    let total: f64 = query_vectors
        .iter()
        .map(|q| {
            let retrieved = search_with_quality(db, q, K, quality);
            let ground_truth = brute_force_cosine_knn(all_vectors, q, K);
            compute_recall(&retrieved, &ground_truth, K)
        })
        .sum();

    total / query_vectors.len() as f64
}

// =========================================================================
// BDD Scenarios
// =========================================================================

#[test]
fn test_recall_contract_fast_mode_gte_90() {
    // GIVEN: a collection with 1K random vectors (64D, cosine)
    let vectors = generate_vectors(NUM_VECTORS, DIM, 42);
    let queries = generate_vectors(NUM_QUERIES, DIM, 123);
    let dir = tempfile::TempDir::new().expect("test: temp dir");
    let db = setup_recall_db(&dir, &vectors);

    // WHEN: searching in Fast mode
    let avg_recall = measure_avg_recall(&db, &queries, &vectors, velesdb_core::SearchQuality::Fast);

    // THEN: recall@10 >= 0.90
    assert!(
        avg_recall >= 0.90,
        "Fast mode recall@{K} should be >= 0.90, got {avg_recall:.3}"
    );
}

#[test]
fn test_recall_contract_balanced_mode_gte_95() {
    // GIVEN: a collection with 1K random vectors (64D, cosine)
    let vectors = generate_vectors(NUM_VECTORS, DIM, 42);
    let queries = generate_vectors(NUM_QUERIES, DIM, 456);
    let dir = tempfile::TempDir::new().expect("test: temp dir");
    let db = setup_recall_db(&dir, &vectors);

    // WHEN: searching in Balanced mode
    let avg_recall = measure_avg_recall(
        &db,
        &queries,
        &vectors,
        velesdb_core::SearchQuality::Balanced,
    );

    // THEN: recall@10 >= 0.95
    assert!(
        avg_recall >= 0.95,
        "Balanced mode recall@{K} should be >= 0.95, got {avg_recall:.3}"
    );
}

#[test]
fn test_recall_contract_accurate_mode_gte_99() {
    // GIVEN: a collection with 1K random vectors (64D, cosine)
    let vectors = generate_vectors(NUM_VECTORS, DIM, 42);
    let queries = generate_vectors(NUM_QUERIES, DIM, 789);
    let dir = tempfile::TempDir::new().expect("test: temp dir");
    let db = setup_recall_db(&dir, &vectors);

    // WHEN: searching in Accurate mode
    let avg_recall = measure_avg_recall(
        &db,
        &queries,
        &vectors,
        velesdb_core::SearchQuality::Accurate,
    );

    // THEN: recall@10 >= 0.99
    assert!(
        avg_recall >= 0.99,
        "Accurate mode recall@{K} should be >= 0.99, got {avg_recall:.3}"
    );
}

#[test]
fn test_recall_contract_perfect_mode_eq_100() {
    // GIVEN: a collection with 1K random vectors (64D, cosine)
    let vectors = generate_vectors(NUM_VECTORS, DIM, 42);
    let queries = generate_vectors(NUM_QUERIES, DIM, 101);
    let dir = tempfile::TempDir::new().expect("test: temp dir");
    let db = setup_recall_db(&dir, &vectors);

    // WHEN: searching in Perfect mode
    let avg_recall = measure_avg_recall(
        &db,
        &queries,
        &vectors,
        velesdb_core::SearchQuality::Perfect,
    );

    // THEN: recall@10 = 1.00
    assert!(
        (avg_recall - 1.0).abs() < f64::EPSILON,
        "Perfect mode recall@{K} should be 1.00, got {avg_recall:.3}"
    );
}
