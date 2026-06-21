//! BDD tests pinning `VelesQL` vector `NEAR` ANN ranking to EXACT golden order.
//!
//! These scenarios golden-pin the ordering that today's suite never asserts:
//! the precise, fully-ordered id sequence returned by `vector NEAR $v`, and
//! that the accompanying scores strictly decrease. The dataset is engineered
//! so the exact (brute-force) nearest-neighbour order is also what HNSW must
//! return, by keeping all points well separated along a single axis.
//!
//! Each scenario follows GIVEN (setup data) -> WHEN (execute SQL) -> THEN
//! (verify exact result), exercising the full pipeline:
//! SQL string -> `Parser::parse()` -> `Database::execute_query()` -> verify.
//!
//! ## Cosine ground truth (query `[1, 0, 0, 0]`, points `[1, off, 0, 0]`)
//!
//! cosine = `1 / sqrt(1 + off^2)`, strictly DECREASING in `off`:
//!
//! | id | vector            | off | cosine similarity |
//! |----|-------------------|-----|-------------------|
//! | 10 | `[1.0, 0.0, 0,0]` | 0.0 | 1.0000            |
//! | 11 | `[1.0, 0.3, 0,0]` | 0.3 | 0.9578            |
//! | 12 | `[1.0, 0.7, 0,0]` | 0.7 | 0.8192            |
//! | 13 | `[1.0, 1.2, 0,0]` | 1.2 | 0.6402            |
//! | 14 | `[1.0, 3.0, 0,0]` | 3.0 | 0.3162            |
//! | 15 | `[0.0, 1.0, 0,0]` |  -  | 0.0000 (orthogonal) |
//!
//! So the exact full order by descending similarity is `[10, 11, 12, 13, 14, 15]`.

use serde_json::json;
use velesdb_core::{Database, DistanceMetric, Point};

use super::helpers::{create_test_db, execute_sql_with_params, vector_param};

// =========================================================================
// Module-specific setup
// =========================================================================

/// Build a `docs` collection (dim 4, cosine) of 6 well-separated points along
/// `[1, off, 0, 0]` plus one orthogonal point, with payloads `{"cat","n"}`.
///
/// Payloads: ids 10,12,14 -> cat "a"; ids 11,13,15 -> cat "b"; `n` == id.
fn setup_docs_cosine(db: &Database) {
    db.create_vector_collection("docs", 4, DistanceMetric::Cosine)
        .expect("test: create docs collection");
    let vc = db
        .get_vector_collection("docs")
        .expect("test: get docs collection");

    vc.upsert(vec![
        Point::new(
            10,
            vec![1.0, 0.0, 0.0, 0.0],
            Some(json!({"cat": "a", "n": 10})),
        ),
        Point::new(
            11,
            vec![1.0, 0.3, 0.0, 0.0],
            Some(json!({"cat": "b", "n": 11})),
        ),
        Point::new(
            12,
            vec![1.0, 0.7, 0.0, 0.0],
            Some(json!({"cat": "a", "n": 12})),
        ),
        Point::new(
            13,
            vec![1.0, 1.2, 0.0, 0.0],
            Some(json!({"cat": "b", "n": 13})),
        ),
        Point::new(
            14,
            vec![1.0, 3.0, 0.0, 0.0],
            Some(json!({"cat": "a", "n": 14})),
        ),
        Point::new(
            15,
            vec![0.0, 1.0, 0.0, 0.0],
            Some(json!({"cat": "b", "n": 15})),
        ),
    ])
    .expect("test: upsert docs corpus");
}

/// Build a `geo` collection (dim 4, Euclidean) with the SAME geometry as
/// `setup_docs_cosine`. With query `[1, 0, 0, 0]`, the Euclidean distance to
/// `[1, off, 0, 0]` is exactly `off`, and to `[0, 1, 0, 0]` is `sqrt(2)`.
///
/// Distances strictly increase: 0.0 < 0.3 < 0.7 < 1.2 < sqrt(2)=1.414 < 3.0.
/// So the nearest-first order is `[20, 21, 22, 23, 25, 24]` (note: orthogonal
/// id 25 at sqrt(2) overtakes id 24 at off=3.0, unlike the cosine case).
fn setup_geo_euclidean(db: &Database) {
    db.create_vector_collection("geo", 4, DistanceMetric::Euclidean)
        .expect("test: create geo collection");
    let vc = db
        .get_vector_collection("geo")
        .expect("test: get geo collection");

    vc.upsert(vec![
        Point::new(
            20,
            vec![1.0, 0.0, 0.0, 0.0],
            Some(json!({"cat": "a", "n": 20})),
        ),
        Point::new(
            21,
            vec![1.0, 0.3, 0.0, 0.0],
            Some(json!({"cat": "b", "n": 21})),
        ),
        Point::new(
            22,
            vec![1.0, 0.7, 0.0, 0.0],
            Some(json!({"cat": "a", "n": 22})),
        ),
        Point::new(
            23,
            vec![1.0, 1.2, 0.0, 0.0],
            Some(json!({"cat": "b", "n": 23})),
        ),
        Point::new(
            24,
            vec![1.0, 3.0, 0.0, 0.0],
            Some(json!({"cat": "a", "n": 24})),
        ),
        Point::new(
            25,
            vec![0.0, 1.0, 0.0, 0.0],
            Some(json!({"cat": "b", "n": 25})),
        ),
    ])
    .expect("test: upsert geo corpus");
}

/// Run a NEAR query against query vector `[1, 0, 0, 0]`, returning ordered ids.
fn near_ids(db: &Database, sql: &str) -> Vec<u64> {
    let results = execute_sql_with_params(db, sql, &vector_param(&[1.0, 0.0, 0.0, 0.0]))
        .expect("test: NEAR query should succeed");
    results.iter().map(|r| r.point.id).collect()
}

// =========================================================================
// Scenario 1: full NEAR order (LIMIT 6) is the exact descending-similarity order
// =========================================================================

/// GIVEN the 6 well-separated cosine docs
/// WHEN `vector NEAR [1,0,0,0] LIMIT 6`
/// THEN the result is the EXACT order `[10, 11, 12, 13, 14, 15]`, because
///      cosine = 1/sqrt(1+off^2) strictly decreases with off (1.0 > 0.958 >
///      0.819 > 0.640 > 0.316 > 0.0), and the points are far enough apart that
///      HNSW reproduces the brute-force order.
#[test]
fn test_near_full_order_limit6() {
    let (_dir, db) = create_test_db();
    setup_docs_cosine(&db);

    let ids = near_ids(&db, "SELECT * FROM docs WHERE vector NEAR $v LIMIT 6;");

    assert_eq!(
        ids,
        vec![10, 11, 12, 13, 14, 15],
        "NEAR must return the exact descending-cosine order"
    );
}

// =========================================================================
// Scenario 2: LIMIT 3 returns exactly the top-3 prefix
// =========================================================================

/// GIVEN the same cosine docs
/// WHEN `vector NEAR [1,0,0,0] LIMIT 3`
/// THEN the result is the EXACT top-3 prefix `[10, 11, 12]` (similarities
///      1.0, 0.958, 0.819 — the three highest), in that order.
#[test]
fn test_near_top3_prefix() {
    let (_dir, db) = create_test_db();
    setup_docs_cosine(&db);

    let ids = near_ids(&db, "SELECT * FROM docs WHERE vector NEAR $v LIMIT 3;");

    assert_eq!(
        ids,
        vec![10, 11, 12],
        "NEAR LIMIT 3 must return exactly the 3 closest, ordered"
    );
}

// =========================================================================
// Scenario 3: scores are strictly decreasing
// =========================================================================

/// GIVEN the cosine docs
/// WHEN `vector NEAR [1,0,0,0] LIMIT 6`
/// THEN the 6 similarity scores are STRICTLY decreasing (no ties), matching the
///      hand-computed sequence 1.0 > 0.958 > 0.819 > 0.640 > 0.316 > 0.0.
#[test]
fn test_near_scores_strictly_decreasing() {
    let (_dir, db) = create_test_db();
    setup_docs_cosine(&db);

    let results = execute_sql_with_params(
        &db,
        "SELECT * FROM docs WHERE vector NEAR $v LIMIT 6;",
        &vector_param(&[1.0, 0.0, 0.0, 0.0]),
    )
    .expect("test: NEAR query should succeed");

    let scores: Vec<f32> = results.iter().map(|r| r.score).collect();
    assert_eq!(scores.len(), 6, "expected 6 scored hits");
    assert!(
        scores.windows(2).all(|w| w[0] > w[1]),
        "scores must be STRICTLY decreasing, got {scores:?}"
    );
}

// =========================================================================
// Scenario 4: NEAR + WHERE cat='b' yields the exact ordered subset
// =========================================================================

/// GIVEN the cosine docs (cat "b" = ids 11, 13, 15)
/// WHEN `vector NEAR [1,0,0,0] AND cat = 'b' LIMIT 6`
/// THEN the result is the EXACT order `[11, 13, 15]` — the cat-"b" rows in the
///      same descending-cosine order (0.958 > 0.640 > 0.0).
#[test]
fn test_near_with_cat_filter_exact_order() {
    let (_dir, db) = create_test_db();
    setup_docs_cosine(&db);

    let ids = near_ids(
        &db,
        "SELECT * FROM docs WHERE vector NEAR $v AND cat = 'b' LIMIT 6;",
    );

    assert_eq!(
        ids,
        vec![11, 13, 15],
        "NEAR + cat='b' must keep descending-cosine order over the subset"
    );
}

// =========================================================================
// Scenario 5: NEAR + WHERE n NOT IN (...) drops listed ids, order preserved
// =========================================================================

/// GIVEN the cosine docs
/// WHEN `vector NEAR [1,0,0,0] AND n NOT IN (11, 14) LIMIT 6`
/// THEN the result is the EXACT order `[10, 12, 13, 15]` — the full ordered
///      sequence `[10,11,12,13,14,15]` with ids 11 and 14 removed, order kept.
#[test]
fn test_near_with_not_in_exact_order() {
    let (_dir, db) = create_test_db();
    setup_docs_cosine(&db);

    let ids = near_ids(
        &db,
        "SELECT * FROM docs WHERE vector NEAR $v AND n NOT IN (11, 14) LIMIT 6;",
    );

    assert_eq!(
        ids,
        vec![10, 12, 13, 15],
        "NOT IN (11,14) must drop those ids and preserve NEAR order"
    );
}

// =========================================================================
// Scenario 6: NEAR + WHERE NOT (n = top_id) drops the top hit, order preserved
// =========================================================================

/// GIVEN the cosine docs (top hit is id 10, cosine 1.0)
/// WHEN `vector NEAR [1,0,0,0] AND NOT (n = 10) LIMIT 6`
/// THEN the result is the EXACT order `[11, 12, 13, 14, 15]` — the full order
///      with the single top hit removed and the remaining order preserved.
#[test]
fn test_near_with_not_eq_drops_top_hit() {
    let (_dir, db) = create_test_db();
    setup_docs_cosine(&db);

    let ids = near_ids(
        &db,
        "SELECT * FROM docs WHERE vector NEAR $v AND NOT (n = 10) LIMIT 6;",
    );

    assert_eq!(
        ids,
        vec![11, 12, 13, 14, 15],
        "NOT (n=10) must drop the top hit and preserve the rest of NEAR order"
    );
}

// =========================================================================
// Scenario 7: Euclidean NEAR full order pins the distance ranking
// =========================================================================

/// GIVEN the Euclidean geo collection (distance to `[1,0,0,0]` == off for the
///       on-axis points, sqrt(2)=1.414 for the orthogonal id 25)
/// WHEN `vector NEAR [1,0,0,0] LIMIT 6`
/// THEN the result is the EXACT nearest-first order `[20, 21, 22, 23, 25, 24]`:
///      distances 0.0 < 0.3 < 0.7 < 1.2 < 1.414 < 3.0, so the orthogonal id 25
///      (1.414) overtakes id 24 (3.0) — the opposite of the cosine case.
#[test]
fn test_near_euclidean_full_order() {
    let (_dir, db) = create_test_db();
    setup_geo_euclidean(&db);

    let ids = near_ids(&db, "SELECT * FROM geo WHERE vector NEAR $v LIMIT 6;");

    assert_eq!(
        ids,
        vec![20, 21, 22, 23, 25, 24],
        "Euclidean NEAR must return the exact ascending-distance order"
    );
}

// =========================================================================
// Scenario 8: Euclidean NEAR + WHERE cat='a' yields the exact ordered subset
// =========================================================================

/// GIVEN the Euclidean geo collection (cat "a" = ids 20, 22, 24)
/// WHEN `vector NEAR [1,0,0,0] AND cat = 'a' LIMIT 6`
/// THEN the result is the EXACT order `[20, 22, 24]` — the cat-"a" rows in
///      ascending-distance order (0.0 < 0.7 < 3.0).
#[test]
fn test_near_euclidean_with_filter_exact_order() {
    let (_dir, db) = create_test_db();
    setup_geo_euclidean(&db);

    let ids = near_ids(
        &db,
        "SELECT * FROM geo WHERE vector NEAR $v AND cat = 'a' LIMIT 6;",
    );

    assert_eq!(
        ids,
        vec![20, 22, 24],
        "Euclidean NEAR + cat='a' must keep ascending-distance order over the subset"
    );
}
