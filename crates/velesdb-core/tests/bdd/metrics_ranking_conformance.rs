//! Exact NEAR-ranking conformance for the three under-tested distance metrics:
//! `DotProduct`, `Hamming`, and `Jaccard`. Cosine and Euclidean already have
//! exact-order coverage in `near_exact_ranking.rs`; the recall floor for
//! Euclidean/DotProduct lives in `recall_contract_multimetric.rs`. NOTHING
//! previously exercised Hamming or Jaccard through a `VelesQL` query, and
//! DotProduct had only a statistical recall test (no pinned order).
//!
//! Every dataset is tiny and hand-computed so the expected ranked id order and
//! the user-visible `SearchResult.score` are deterministic ground truth.
//! Sort directions (authoritative, `distance.rs` `higher_is_better`):
//!   Hamming  = distance   -> ASCENDING  (lower count first)
//!   Jaccard  = similarity -> DESCENDING (higher similarity first)
//!   DotProduct = similarity -> DESCENDING (higher raw dot first)

use velesdb_core::{Database, DistanceMetric, Point};

use super::helpers::{approx_eq, create_test_db, execute_sql_with_params, vector_param};

/// Run `SELECT * FROM <coll> WHERE vector NEAR $v ...` and return (id, score)
/// pairs in result order.
fn ranked(db: &Database, sql: &str, query: &[f32]) -> Vec<(u64, f32)> {
    let params = vector_param(query);
    let results = execute_sql_with_params(db, sql, &params).expect("test: execute NEAR query");
    results.iter().map(|r| (r.point.id, r.score)).collect()
}

/// Just the ids, in result order.
fn ids(pairs: &[(u64, f32)]) -> Vec<u64> {
    pairs.iter().map(|(id, _)| *id).collect()
}

// ============================================================================
// Hamming — distance, ascending (lower differing-bit count = nearer)
// ============================================================================

/// Builds a dim-4 binary (0.0/1.0) Hamming collection. Bit = component > 0.5.
fn setup_hamming(db: &Database) {
    db.create_vector_collection("ham", 4, DistanceMetric::Hamming)
        .expect("test: create hamming collection");
    let vc = db.get_vector_collection("ham").expect("test: get ham");
    vc.upsert(vec![
        Point::new(
            1,
            vec![1.0, 0.0, 1.0, 0.0],
            Some(serde_json::json!({"cat": "a"})),
        ),
        Point::new(
            2,
            vec![1.0, 1.0, 1.0, 1.0],
            Some(serde_json::json!({"cat": "b"})),
        ),
        Point::new(
            3,
            vec![0.0, 0.0, 1.0, 0.0],
            Some(serde_json::json!({"cat": "a"})),
        ),
        Point::new(
            4,
            vec![0.0, 1.0, 0.0, 1.0],
            Some(serde_json::json!({"cat": "b"})),
        ),
    ])
    .expect("test: upsert ham");
}

/// GIVEN q=[1,0,1,0]. Differing-bit counts: id1=0, id3=1, id2=2, id4=4.
/// THEN NEAR returns ascending-distance order [1,3,2,4] with scores 0,1,2,4.
#[test]
fn test_hamming_exact_ascending_order_and_counts() {
    let (_dir, db) = create_test_db();
    setup_hamming(&db);
    let pairs = ranked(
        &db,
        "SELECT * FROM ham WHERE vector NEAR $v LIMIT 4",
        &[1.0, 0.0, 1.0, 0.0],
    );
    assert_eq!(
        ids(&pairs),
        vec![1, 3, 2, 4],
        "ascending differing-bit count"
    );
    let scores: Vec<f32> = pairs.iter().map(|(_, s)| *s).collect();
    for (got, want) in scores.iter().zip([0.0, 1.0, 2.0, 4.0]) {
        assert!(approx_eq(*got, want, 1e-6), "hamming score {got} != {want}");
    }
}

/// LIMIT 2 keeps the two smallest counts (id1=0, id3=1) — k applies after sort.
#[test]
fn test_hamming_limit_prefix() {
    let (_dir, db) = create_test_db();
    setup_hamming(&db);
    let pairs = ranked(
        &db,
        "SELECT * FROM ham WHERE vector NEAR $v LIMIT 2",
        &[1.0, 0.0, 1.0, 0.0],
    );
    assert_eq!(ids(&pairs), vec![1, 3], "two nearest by ascending count");
}

/// Scores are STRICTLY INCREASING — pins the ascending (distance) direction;
/// would fail if Hamming were ever sorted as a similarity.
#[test]
fn test_hamming_scores_strictly_increasing() {
    let (_dir, db) = create_test_db();
    setup_hamming(&db);
    let pairs = ranked(
        &db,
        "SELECT * FROM ham WHERE vector NEAR $v LIMIT 4",
        &[1.0, 0.0, 1.0, 0.0],
    );
    let scores: Vec<f32> = pairs.iter().map(|(_, s)| *s).collect();
    assert!(
        scores.windows(2).all(|w| w[0] < w[1]),
        "ascending: {scores:?}"
    );
}

/// NEAR + scalar filter: cat='a' = {id1,id3}; ascending count 0<1 -> [1,3].
#[test]
fn test_hamming_near_with_filter() {
    let (_dir, db) = create_test_db();
    setup_hamming(&db);
    let pairs = ranked(
        &db,
        "SELECT * FROM ham WHERE vector NEAR $v AND cat = 'a' LIMIT 4",
        &[1.0, 0.0, 1.0, 0.0],
    );
    assert_eq!(
        ids(&pairs),
        vec![1, 3],
        "filtered ascending count over cat='a'"
    );
}

// ============================================================================
// Jaccard — similarity (Ruzicka Σmin/Σmax), descending (higher = nearer)
// ============================================================================

/// Builds a dim-5 binary (0.0/1.0) Jaccard collection.
fn setup_jaccard(db: &Database) {
    db.create_vector_collection("jac", 5, DistanceMetric::Jaccard)
        .expect("test: create jaccard collection");
    let vc = db.get_vector_collection("jac").expect("test: get jac");
    vc.upsert(vec![
        Point::new(
            1,
            vec![1.0, 1.0, 1.0, 1.0, 0.0],
            Some(serde_json::json!({"cat": "a"})),
        ),
        Point::new(
            2,
            vec![1.0, 1.0, 1.0, 0.0, 0.0],
            Some(serde_json::json!({"cat": "b"})),
        ),
        Point::new(
            3,
            vec![1.0, 1.0, 0.0, 0.0, 1.0],
            Some(serde_json::json!({"cat": "a"})),
        ),
        Point::new(
            4,
            vec![0.0, 0.0, 0.0, 0.0, 1.0],
            Some(serde_json::json!({"cat": "b"})),
        ),
    ])
    .expect("test: upsert jac");
}

/// GIVEN q=[1,1,1,1,0]. Σmin/Σmax: id1=4/4=1.0, id2=3/4=0.75, id3=2/5=0.40,
/// id4=0/5=0.0. THEN NEAR returns descending-similarity order [1,2,3,4].
#[test]
fn test_jaccard_exact_descending_order_and_similarities() {
    let (_dir, db) = create_test_db();
    setup_jaccard(&db);
    let pairs = ranked(
        &db,
        "SELECT * FROM jac WHERE vector NEAR $v LIMIT 4",
        &[1.0, 1.0, 1.0, 1.0, 0.0],
    );
    assert_eq!(
        ids(&pairs),
        vec![1, 2, 3, 4],
        "descending Jaccard similarity"
    );
    let scores: Vec<f32> = pairs.iter().map(|(_, s)| *s).collect();
    for (got, want) in scores.iter().zip([1.0, 0.75, 0.40, 0.0]) {
        assert!(approx_eq(*got, want, 1e-6), "jaccard score {got} != {want}");
    }
}

/// LIMIT 2 keeps the two most similar (id1=1.0, id2=0.75).
#[test]
fn test_jaccard_limit_prefix() {
    let (_dir, db) = create_test_db();
    setup_jaccard(&db);
    let pairs = ranked(
        &db,
        "SELECT * FROM jac WHERE vector NEAR $v LIMIT 2",
        &[1.0, 1.0, 1.0, 1.0, 0.0],
    );
    assert_eq!(ids(&pairs), vec![1, 2], "two highest Jaccard similarity");
}

/// Scores STRICTLY DECREASING — pins descending (similarity) direction; would
/// fail if Jaccard leaked the internal 1-sim distance while keeping desc sort.
#[test]
fn test_jaccard_scores_strictly_decreasing() {
    let (_dir, db) = create_test_db();
    setup_jaccard(&db);
    let pairs = ranked(
        &db,
        "SELECT * FROM jac WHERE vector NEAR $v LIMIT 4",
        &[1.0, 1.0, 1.0, 1.0, 0.0],
    );
    let scores: Vec<f32> = pairs.iter().map(|(_, s)| *s).collect();
    assert!(
        scores.windows(2).all(|w| w[0] > w[1]),
        "descending: {scores:?}"
    );
}

/// NEAR + scalar filter: cat='b' = {id2,id4}; descending sim 0.75>0.0 -> [2,4].
#[test]
fn test_jaccard_near_with_filter() {
    let (_dir, db) = create_test_db();
    setup_jaccard(&db);
    let pairs = ranked(
        &db,
        "SELECT * FROM jac WHERE vector NEAR $v AND cat = 'b' LIMIT 4",
        &[1.0, 1.0, 1.0, 1.0, 0.0],
    );
    assert_eq!(
        ids(&pairs),
        vec![2, 4],
        "filtered descending sim over cat='b'"
    );
}

// ============================================================================
// DotProduct — similarity, descending (higher raw dot = nearer)
// ============================================================================

/// Builds a dim-2 NON-normalized DotProduct collection so dot-order differs
/// from cosine-order (proving the ranking is really raw dot, not cosine).
fn setup_dot(db: &Database) {
    db.create_vector_collection("dot", 2, DistanceMetric::DotProduct)
        .expect("test: create dot collection");
    let vc = db.get_vector_collection("dot").expect("test: get dot");
    vc.upsert(vec![
        Point::new(1, vec![2.0, 0.0], Some(serde_json::json!({"cat": "a"}))),
        Point::new(2, vec![5.0, 3.0], Some(serde_json::json!({"cat": "b"}))),
        Point::new(3, vec![0.95, 0.05], Some(serde_json::json!({"cat": "a"}))),
        Point::new(4, vec![1.0, 4.0], Some(serde_json::json!({"cat": "b"}))),
    ])
    .expect("test: upsert dot");
}

/// GIVEN q=[1,0]. Raw dot: id2=5.0, id1=2.0, id4=1.0, id3=0.95. THEN NEAR
/// returns descending-dot order [2,1,4,3] with the POSITIVE raw dot as score.
/// Cosine order would be [1,3,2,4] (id1 first, cosine 1.0) — different at every
/// position, so this order fails if the engine ever fell back to cosine.
#[test]
fn test_dotproduct_exact_descending_order_not_cosine() {
    let (_dir, db) = create_test_db();
    setup_dot(&db);
    let pairs = ranked(
        &db,
        "SELECT * FROM dot WHERE vector NEAR $v LIMIT 4",
        &[1.0, 0.0],
    );
    assert_eq!(
        ids(&pairs),
        vec![2, 1, 4, 3],
        "descending raw dot (not cosine)"
    );
    let scores: Vec<f32> = pairs.iter().map(|(_, s)| *s).collect();
    for (got, want) in scores.iter().zip([5.0, 2.0, 1.0, 0.95]) {
        assert!(approx_eq(*got, want, 1e-5), "dot score {got} != {want}");
    }
}

/// LIMIT 1 keeps the largest raw dot (id2=5.0) — cheapest dot-not-cosine guard.
#[test]
fn test_dotproduct_top1_is_highest_dot() {
    let (_dir, db) = create_test_db();
    setup_dot(&db);
    let pairs = ranked(
        &db,
        "SELECT * FROM dot WHERE vector NEAR $v LIMIT 1",
        &[1.0, 0.0],
    );
    assert_eq!(ids(&pairs), vec![2], "top-1 is the highest raw dot");
}

/// Scores STRICTLY DECREASING (5.0>2.0>1.0>0.95) — pins descending + positive dot.
#[test]
fn test_dotproduct_scores_strictly_decreasing() {
    let (_dir, db) = create_test_db();
    setup_dot(&db);
    let pairs = ranked(
        &db,
        "SELECT * FROM dot WHERE vector NEAR $v LIMIT 4",
        &[1.0, 0.0],
    );
    let scores: Vec<f32> = pairs.iter().map(|(_, s)| *s).collect();
    assert!(
        scores.windows(2).all(|w| w[0] > w[1]),
        "descending: {scores:?}"
    );
}

/// NEAR + scalar filter: cat='b' = {id2,id4}; descending dot 5.0>1.0 -> [2,4].
#[test]
fn test_dotproduct_near_with_filter() {
    let (_dir, db) = create_test_db();
    setup_dot(&db);
    let pairs = ranked(
        &db,
        "SELECT * FROM dot WHERE vector NEAR $v AND cat = 'b' LIMIT 4",
        &[1.0, 0.0],
    );
    assert_eq!(
        ids(&pairs),
        vec![2, 4],
        "filtered descending dot over cat='b'"
    );
}
