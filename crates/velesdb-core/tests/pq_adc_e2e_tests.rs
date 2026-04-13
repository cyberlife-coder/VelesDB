#![cfg(feature = "persistence")]
//! E2E tests for the PQ/ADC search pipeline.
//! Verifies the complete path: collection -> train -> insert -> search -> recall.
//!
//! These tests exercise the **full production pipeline**: `Database::open()` ->
//! `create_collection()` -> `upsert()` -> `TRAIN QUANTIZER` (VelesQL) ->
//! `search()` / `search_ids()` -> recall measurement against brute-force ground truth.

#![allow(clippy::cast_precision_loss, clippy::doc_markdown)]

use std::collections::{HashMap, HashSet};

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use tempfile::TempDir;
use velesdb_core::velesql::Parser;
use velesdb_core::{Database, DistanceMetric, Point, StorageMode};

// ---------------------------------------------------------------------------
// Helpers (deterministic RNG, brute-force ground truth, recall)
// ---------------------------------------------------------------------------

/// Generates `n` random vectors of dimension `dim` in `[-1, 1]` using a seeded RNG.
fn generate_vectors(n: usize, dim: usize, seed: u64) -> Vec<Vec<f32>> {
    let mut rng = StdRng::seed_from_u64(seed);
    (0..n)
        .map(|_| (0..dim).map(|_| rng.gen_range(-1.0_f32..1.0)).collect())
        .collect()
}

/// Normalizes a vector to unit length (for cosine metric ground truth).
fn normalize(v: &[f32]) -> Vec<f32> {
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm < f32::EPSILON {
        return v.to_vec();
    }
    v.iter().map(|x| x / norm).collect()
}

/// Cosine distance: `1 - cos_sim`. Lower is better.
fn cosine_distance(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    let denom = norm_a * norm_b;
    if denom < f32::EPSILON {
        return 1.0;
    }
    1.0 - (dot / denom)
}

/// Brute-force exact top-k using cosine distance. Returns sorted IDs.
fn brute_force_cosine(query: &[f32], dataset: &[Vec<f32>], k: usize) -> Vec<u64> {
    let mut dists: Vec<(u64, f32)> = dataset
        .iter()
        .enumerate()
        .map(|(i, v)| {
            #[allow(clippy::cast_possible_truncation)]
            let id = i as u64;
            (id, cosine_distance(query, v))
        })
        .collect();
    dists.sort_by(|a, b| a.1.total_cmp(&b.1));
    dists.iter().take(k).map(|(id, _)| *id).collect()
}

/// Computes recall@k between retrieved IDs and ground truth IDs.
#[allow(clippy::cast_precision_loss)]
fn recall_at_k(ground_truth: &[u64], results: &[u64], k: usize) -> f64 {
    let gt_set: HashSet<u64> = ground_truth.iter().take(k).copied().collect();
    let result_set: HashSet<u64> = results.iter().take(k).copied().collect();
    let intersection = gt_set.intersection(&result_set).count();
    intersection as f64 / k as f64
}

/// Creates a fresh Database and returns (`TempDir`, `Database`).
/// `TempDir` must be held alive for the database lifetime.
fn create_test_db() -> (TempDir, Database) {
    let dir = TempDir::new().expect("test: create temp dir");
    let db = Database::open(dir.path()).expect("test: open database");
    (dir, db)
}

/// Executes a VelesQL statement through the full pipeline.
fn execute_sql(db: &Database, sql: &str) -> velesdb_core::Result<Vec<velesdb_core::SearchResult>> {
    let query = Parser::parse(sql).map_err(|e| velesdb_core::Error::Query(e.to_string()))?;
    db.execute_query(&query, &HashMap::new())
}

/// Inserts `vectors` into collection `name` with sequential IDs starting at 0.
fn insert_vectors(db: &Database, name: &str, vectors: &[Vec<f32>]) {
    let coll = db
        .get_vector_collection(name)
        .expect("test: collection must exist");
    let points: Vec<Point> = vectors
        .iter()
        .enumerate()
        .map(|(i, v)| {
            #[allow(clippy::cast_possible_truncation)]
            let id = i as u64;
            Point::new(id, v.clone(), Some(serde_json::json!({})))
        })
        .collect();
    coll.upsert(points).expect("test: upsert must succeed");
}

// ---------------------------------------------------------------------------
// Test 1: PQ train + search recall
// ---------------------------------------------------------------------------

/// GIVEN: A collection with 1000 cosine vectors of dimension 32
/// WHEN:  Train PQ (m=8, k=64) via VelesQL, then search 10 queries (top_k=10)
/// THEN:  Average recall@10 >= 0.70 AND all results have valid IDs and scores > 0
#[test]
fn test_pq_train_and_search_maintains_recall() {
    let (_dir, db) = create_test_db();

    // GIVEN: create collection, insert 1000 vectors
    db.create_collection("pq_recall", 32, DistanceMetric::Cosine)
        .expect("test: create collection");

    let dataset = generate_vectors(1000, 32, 42);
    insert_vectors(&db, "pq_recall", &dataset);

    // WHEN: train PQ quantizer
    // Use k=64 (centroids) because k must not exceed training vector count.
    // m=8 subvectors for dim=32 means subspace_dim=4.
    execute_sql(&db, "TRAIN QUANTIZER ON pq_recall WITH (m=8, k=64)")
        .expect("test: TRAIN QUANTIZER must succeed");

    // Verify storage mode changed
    let coll = db
        .get_vector_collection("pq_recall")
        .expect("test: collection must exist after training");
    assert_eq!(
        coll.config().storage_mode,
        StorageMode::ProductQuantization,
        "storage mode must be ProductQuantization after training"
    );

    // WHEN: search with 10 random queries
    let queries = generate_vectors(10, 32, 123);
    let k = 10;
    let mut total_recall = 0.0;

    for query in &queries {
        // Normalize for cosine metric
        let normalized_query = normalize(query);

        let results = coll
            .search(&normalized_query, k)
            .expect("test: PQ search must succeed");

        // THEN: all results have valid IDs and non-negative scores
        assert!(
            !results.is_empty(),
            "PQ search must return at least one result"
        );
        for r in &results {
            assert!(
                r.score >= 0.0,
                "PQ search score must be non-negative, got {}",
                r.score
            );
            assert!(
                r.point.id < 1000,
                "result ID {} must be within dataset range",
                r.point.id
            );
        }

        // Compute recall against brute-force ground truth
        let gt = brute_force_cosine(&normalized_query, &dataset, k);
        let result_ids: Vec<u64> = results.iter().map(|r| r.point.id).collect();
        total_recall += recall_at_k(&gt, &result_ids, k);
    }

    #[allow(clippy::cast_precision_loss)]
    let avg_recall = total_recall / queries.len() as f64;

    assert!(
        avg_recall >= 0.70,
        "PQ recall@{k} must be >= 0.70, got {avg_recall:.4}"
    );
}

// ---------------------------------------------------------------------------
// Test 2: PQ search returns known vector in top results
// ---------------------------------------------------------------------------

/// GIVEN: A collection with 100 vectors, including one identical to the query
/// WHEN:  Train PQ and search with that exact vector
/// THEN:  The identical vector appears in top-3 results
#[test]
fn test_pq_search_returns_correct_results_for_known_vectors() {
    let (_dir, db) = create_test_db();

    // GIVEN: 100 vectors of dimension 32 (euclidean for simpler distance semantics)
    db.create_collection("pq_known", 32, DistanceMetric::Euclidean)
        .expect("test: create collection");

    let mut dataset = generate_vectors(100, 32, 99);

    // Plant a known vector at index 42
    let known_vector: Vec<f32> = (0..32).map(|i| (i as f32) * 0.1).collect();
    dataset[42] = known_vector.clone();

    insert_vectors(&db, "pq_known", &dataset);

    // WHEN: train PQ (m=4, k=32 since we only have 100 vectors)
    execute_sql(&db, "TRAIN QUANTIZER ON pq_known WITH (m=4, k=32)")
        .expect("test: TRAIN QUANTIZER must succeed");

    let coll = db
        .get_vector_collection("pq_known")
        .expect("test: collection must exist");

    // Search with the exact known vector
    let results = coll
        .search(&known_vector, 10)
        .expect("test: search with known vector must succeed");

    assert!(
        !results.is_empty(),
        "search must return at least one result"
    );

    // THEN: the identical vector (id=42) must appear in top-3
    let top_ids: Vec<u64> = results.iter().take(3).map(|r| r.point.id).collect();
    assert!(
        top_ids.contains(&42),
        "known vector (id=42) must appear in top-3 results, got top-3: {top_ids:?}"
    );
}

// ---------------------------------------------------------------------------
// Test 3: PQ vs full precision result overlap
// ---------------------------------------------------------------------------

/// GIVEN: A collection with 500 euclidean vectors of dimension 64
/// WHEN:  Search the same query with PQ mode and full mode
/// THEN:  At least 7/10 of the top-10 results overlap (70% overlap)
#[test]
fn test_pq_adc_batch_rescore_matches_full_precision_ordering() {
    // Build full-precision collection
    let (dir_full, db_full) = create_test_db();
    db_full
        .create_collection("full_ref", 64, DistanceMetric::Euclidean)
        .expect("test: create full collection");

    let dataset = generate_vectors(500, 64, 77);
    insert_vectors(&db_full, "full_ref", &dataset);

    // Build PQ collection (same data)
    let (_dir_pq, db_pq) = create_test_db();
    db_pq
        .create_collection("pq_compare", 64, DistanceMetric::Euclidean)
        .expect("test: create PQ collection");
    insert_vectors(&db_pq, "pq_compare", &dataset);

    // Train PQ on the second collection
    // m=8 subvectors for dim=64 means subspace_dim=8.
    // k=64 centroids (well under 500 vectors).
    execute_sql(&db_pq, "TRAIN QUANTIZER ON pq_compare WITH (m=8, k=64)")
        .expect("test: TRAIN QUANTIZER must succeed");

    let coll_full = db_full
        .get_vector_collection("full_ref")
        .expect("test: full collection");
    let coll_pq = db_pq
        .get_vector_collection("pq_compare")
        .expect("test: PQ collection");

    // Verify PQ is active
    assert_eq!(
        coll_pq.config().storage_mode,
        StorageMode::ProductQuantization,
    );

    // Search with 5 random queries and measure overlap
    let queries = generate_vectors(5, 64, 200);
    let k = 10;
    let mut total_overlap = 0_usize;
    let total_expected = queries.len() * k;

    for query in &queries {
        let full_results = coll_full
            .search(query, k)
            .expect("test: full search must succeed");
        let pq_results = coll_pq
            .search(query, k)
            .expect("test: PQ search must succeed");

        let full_ids: HashSet<u64> = full_results.iter().map(|r| r.point.id).collect();
        let pq_ids: HashSet<u64> = pq_results.iter().map(|r| r.point.id).collect();
        total_overlap += full_ids.intersection(&pq_ids).count();
    }

    #[allow(clippy::cast_precision_loss)]
    let overlap_ratio = total_overlap as f64 / total_expected as f64;

    assert!(
        overlap_ratio >= 0.70,
        "PQ vs full-precision top-{k} overlap must be >= 70%, got {:.1}% ({total_overlap}/{total_expected})",
        overlap_ratio * 100.0
    );

    // Keep TempDirs alive until assertions complete
    drop(dir_full);
}

// ---------------------------------------------------------------------------
// Test 4: PQ on empty collection
// ---------------------------------------------------------------------------

/// GIVEN: A collection with PQ storage mode but zero vectors
/// WHEN:  Attempt to train PQ and search
/// THEN:  Training returns an appropriate error, search returns empty, no crash
#[test]
fn test_pq_empty_collection_search_returns_empty() {
    let (_dir, db) = create_test_db();

    db.create_collection("pq_empty", 16, DistanceMetric::Euclidean)
        .expect("test: create empty collection");

    // Attempt to train on empty collection — should return a TrainingFailed error
    // because there are no vectors to train on.
    let train_result = execute_sql(&db, "TRAIN QUANTIZER ON pq_empty WITH (m=4, k=8)");
    assert!(
        train_result.is_err(),
        "training PQ on an empty collection must fail"
    );

    // Search on the (still Full-mode) collection must return empty, not crash
    let coll = db
        .get_vector_collection("pq_empty")
        .expect("test: collection must exist");
    let query = vec![0.0_f32; 16];
    let results = coll
        .search(&query, 10)
        .expect("test: search on empty collection must not crash");
    assert!(
        results.is_empty(),
        "search on empty collection must return empty results"
    );
}
