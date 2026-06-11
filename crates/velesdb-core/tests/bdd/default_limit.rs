//! BDD tests for the implicit SELECT LIMIT contract.
//!
//! Every SELECT without an explicit LIMIT clause returns at most
//! `DEFAULT_SELECT_LIMIT` (10) rows — vector NEAR, scalar filter, and hybrid
//! forms alike. Two documented exceptions are frozen here:
//! `MATCH ... RETURN` (all matches) and compound queries
//! (UNION/INTERSECT/EXCEPT — set operation over full operands).
//!
//! All tests exercise the full pipeline: SQL string -> parse -> validate ->
//! execute -> verify.

use serde_json::json;
use velesdb_core::{Database, Point};

use super::helpers::{create_test_db, execute_sql, execute_sql_with_params, vector_param};

// =========================================================================
// Module-specific setup
// =========================================================================

/// Creates a "corpus" collection of 20 points that all match `kind = 'bulk'`
/// and carry the `Item` label, with decreasing similarity to `[1, 0, 0, 0]`.
fn setup_corpus(db: &Database) {
    db.create_vector_collection("corpus", 4, velesdb_core::DistanceMetric::Cosine)
        .expect("test: create corpus collection");
    let vc = db
        .get_vector_collection("corpus")
        .expect("test: get corpus collection");

    let points: Vec<Point> = (1..=20u8)
        .map(|i| {
            Point::new(
                u64::from(i),
                vec![1.0, f32::from(i) * 0.01, 0.0, 0.0],
                Some(json!({"_labels": ["Item"], "kind": "bulk"})),
            )
        })
        .collect();
    vc.upsert(points).expect("test: upsert corpus");
}

/// Creates two disjoint collections (`left_set` ids 1..=8, `right_set`
/// ids 11..=18) so a UNION yields 16 distinct rows.
fn setup_disjoint_collections(db: &Database) {
    for (name, base) in [("left_set", 0u64), ("right_set", 10)] {
        db.create_vector_collection(name, 4, velesdb_core::DistanceMetric::Cosine)
            .expect("test: create set-op collection");
        let vc = db
            .get_vector_collection(name)
            .expect("test: get set-op collection");
        let points: Vec<Point> = (1..=8u64)
            .map(|i| {
                Point::new(
                    base + i,
                    vec![1.0, 0.0, 0.0, 0.0],
                    Some(json!({"kind": "bulk"})),
                )
            })
            .collect();
        vc.upsert(points).expect("test: upsert set-op collection");
    }
}

// =========================================================================
// A. Default applies: every plain SELECT without LIMIT caps at 10
// =========================================================================

/// GIVEN 20 points all close to the query vector
/// WHEN running `SELECT * ... WHERE vector NEAR $v` without LIMIT
/// THEN exactly 10 rows are returned (engine default LIMIT).
#[test]
fn test_near_without_limit_defaults_to_10() {
    let (_dir, db) = create_test_db();
    setup_corpus(&db);

    let results = execute_sql_with_params(
        &db,
        "SELECT * FROM corpus WHERE vector NEAR $v",
        &vector_param(&[1.0, 0.0, 0.0, 0.0]),
    )
    .expect("NEAR without LIMIT must execute");

    assert_eq!(results.len(), 10, "default LIMIT 10 must cap NEAR results");
}

/// GIVEN 20 points all matching `kind = 'bulk'`
/// WHEN running a pure scalar SELECT without LIMIT
/// THEN exactly 10 rows are returned (engine default LIMIT).
#[test]
fn test_scalar_filter_without_limit_defaults_to_10() {
    let (_dir, db) = create_test_db();
    setup_corpus(&db);

    let results = execute_sql(&db, "SELECT * FROM corpus WHERE kind = 'bulk'")
        .expect("scalar SELECT without LIMIT must execute");

    assert_eq!(
        results.len(),
        10,
        "default LIMIT 10 must cap scalar filter results"
    );
}

// =========================================================================
// B. Documented exceptions: MATCH ... RETURN and compound queries
// =========================================================================

/// GIVEN 20 nodes labeled `Item`
/// WHEN running `MATCH (n:Item) RETURN n` without LIMIT
/// THEN all 20 matches are returned (MATCH has no implicit limit).
#[test]
fn test_match_return_without_limit_returns_all_matches() {
    let (_dir, db) = create_test_db();
    setup_corpus(&db);

    let mut params = std::collections::HashMap::new();
    params.insert("_collection".to_string(), json!("corpus"));
    let results = execute_sql_with_params(&db, "MATCH (n:Item) RETURN n", &params)
        .expect("MATCH without LIMIT must execute");

    assert_eq!(
        results.len(),
        20,
        "MATCH ... RETURN has no implicit limit: all matches expected"
    );
}

/// GIVEN two disjoint collections of 8 rows each
/// WHEN running `SELECT ... UNION SELECT ...` without LIMIT
/// THEN all 16 rows survive (compound queries have no implicit limit).
#[test]
fn test_union_without_limit_keeps_all_rows() {
    let (_dir, db) = create_test_db();
    setup_disjoint_collections(&db);

    let results = execute_sql(&db, "SELECT * FROM left_set UNION SELECT * FROM right_set")
        .expect("UNION without LIMIT must execute");

    assert_eq!(
        results.len(),
        16,
        "compound queries have no implicit limit: full union expected"
    );
}
