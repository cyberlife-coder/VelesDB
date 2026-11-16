//! BDD scenarios: a `similarity()` threshold is a filter over the whole
//! collection, read the same whatever shape the WHERE takes.
//!
//! The rows are every point the WHERE holds for, ranked by the first
//! `similarity()` (most similar first, so closest first under a distance),
//! each scored by it. Before, the threshold filtered an approximate window of
//! nearest points: a selective metadata filter or a named vector left rows
//! out, the NOT path answered in id order, a leaf on a named vector was read
//! against the point's own vector, and a union scored its metadata side 1.0.

use std::collections::HashMap;

use serde_json::json;
use velesdb_core::{Database, DistanceMetric, Point};

use super::helpers::{create_test_db, execute_sql_with_params};

/// `s` (dim 2, cosine) and its similarity to `$x = [1, 0]`:
/// - ids 1..=100, `kind = 'near'`: `[1, 0.01·id]`, from 0.99995 down to
///   0.707; their named vector `alt = [0, 1]` is orthogonal to `$x`;
/// - ids 101..=105, `kind = 'mid'`: `[1, 1.1 + 0.02·(106 − id)]`, from 0.640
///   (101) up to 0.666 (105); `alt = [1, 0.01·(id − 100)]`, closest for 101.
fn setup(db: &Database) {
    db.create_vector_collection("s", 2, DistanceMetric::Cosine)
        .expect("test: create s");
    let s = db.get_vector_collection("s").expect("test: s");
    let points: Vec<Point> = (1..=100_u64)
        .map(|id| {
            Point::new(
                id,
                vec![1.0, 0.01 * id as f32],
                Some(json!({"kind": "near", "alt": [0.0, 1.0]})),
            )
        })
        .chain((101..=105_u64).map(|id| {
            Point::new(
                id,
                vec![1.0, 1.1 + 0.02 * (106 - id) as f32],
                Some(json!({"kind": "mid", "alt": [1.0, 0.01 * (id - 100) as f32]})),
            )
        }))
        .collect();
    s.upsert(points).expect("test: upsert");
}

fn params() -> HashMap<String, serde_json::Value> {
    HashMap::from([
        ("x".to_string(), json!([1.0, 0.0])),
        ("o".to_string(), json!([0.0, 0.0])),
    ])
}

fn ids(sql: &str, db: &Database) -> Vec<u64> {
    execute_sql_with_params(db, sql, &params())
        .unwrap_or_else(|e| panic!("test: {sql}: {e}"))
        .iter()
        .map(|r| r.point.id)
        .collect()
}

/// GIVEN 100 points more similar to `$x` than the 5 that pass the filter
/// WHEN a threshold every point passes is AND-ed with that filter, LIMIT 4
/// THEN the 4 most similar passing points come back, not none.
#[test]
fn a_selective_filter_still_fills_the_limit() {
    let (_dir, db) = create_test_db();
    setup(&db);
    assert_eq!(
        ids(
            "SELECT * FROM s WHERE similarity(vector, $x) > 0.5 AND kind = 'mid' LIMIT 4",
            &db
        ),
        vec![105, 104, 103, 102]
    );
}

/// GIVEN points whose own vectors are near `$x` but whose `alt` is not
/// WHEN the threshold is on `alt`
/// THEN the points whose `alt` passes come back, ranked by `alt`.
#[test]
fn a_named_vector_threshold_reads_the_named_vector() {
    let (_dir, db) = create_test_db();
    setup(&db);
    assert_eq!(
        ids(
            "SELECT * FROM s WHERE similarity(alt, $x) > 0.9 LIMIT 3",
            &db
        ),
        vec![101, 102, 103]
    );
}

/// GIVEN the points below 0.68 of `$x`
/// WHEN `NOT similarity() > 0.68` asks for 3
/// THEN the 3 most similar of them come back, best first — not the 3
///      lowest ids.
#[test]
fn a_negated_threshold_ranks_by_similarity() {
    let (_dir, db) = create_test_db();
    setup(&db);
    assert_eq!(
        ids(
            "SELECT * FROM s WHERE NOT similarity(vector, $x) > 0.68 LIMIT 3",
            &db
        ),
        vec![105, 104, 103]
    );
}

/// GIVEN two thresholds on different vectors, one under NOT
/// WHEN both must hold
/// THEN each is read on its own vector (103, at 0.653, fails the first), the
///      rows ranked by the first.
#[test]
fn each_threshold_reads_its_own_vector() {
    let (_dir, db) = create_test_db();
    setup(&db);
    assert_eq!(
        ids(
            "SELECT * FROM s WHERE NOT similarity(vector, $x) > 0.65 \
             AND similarity(alt, $x) > 0.99 LIMIT 10",
            &db
        ),
        vec![102, 101]
    );
}

/// GIVEN a distance collection
/// WHEN a threshold is OR-ed with a metadata predicate
/// THEN every row is scored by its real distance and ranked closest first —
///      the metadata side is not a fabricated 1.0 sorted as a similarity.
#[test]
fn a_union_ranks_by_the_real_score_in_the_metric_direction() {
    let (_dir, db) = create_test_db();
    db.create_vector_collection("u", 2, DistanceMetric::Euclidean)
        .expect("test: create u");
    let u = db.get_vector_collection("u").expect("test: u");
    u.upsert(vec![
        Point::new(1, vec![9.0, 0.0], Some(json!({"kind": "y"}))),
        Point::new(2, vec![5.0, 0.0], Some(json!({"kind": "x"}))),
        Point::new(3, vec![1.0, 0.0], Some(json!({"kind": "b"}))),
        Point::new(4, vec![0.0, 0.0], Some(json!({"kind": "a"}))),
    ])
    .expect("test: upsert");
    let rows = execute_sql_with_params(
        &db,
        "SELECT * FROM u WHERE similarity(vector, $o) > 2.0 OR kind = 'x' LIMIT 10",
        &params(),
    )
    .expect("test: union");
    let got: Vec<(u64, f32)> = rows.iter().map(|r| (r.point.id, r.score)).collect();
    assert_eq!(got, vec![(4, 0.0), (3, 1.0), (2, 5.0)]);
}
