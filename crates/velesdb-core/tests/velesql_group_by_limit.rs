#![cfg(feature = "persistence")]
//! Reproducer for issue #1556: `Database::execute_aggregate` must apply the
//! statement's `OFFSET`/`LIMIT` to GROUP BY results after ORDER BY, matching
//! the WASM aggregate pipeline (`finalize_aggregated` routes group rows
//! through `apply_limit_offset` with no default cap — a LIMIT-less GROUP BY
//! still returns every group).

use std::collections::HashMap;

use serde_json::{json, Value};
use tempfile::TempDir;
use velesdb_core::velesql::Parser;
use velesdb_core::{Database, DistanceMetric, Point};

/// Three groups by construction: tech (3 rows), other (2 rows), misc (1 row).
fn open_grouped_db() -> (TempDir, Database) {
    let dir = TempDir::new().expect("tempdir");
    let db = Database::open(dir.path()).expect("open db");
    db.create_collection("docs", 2, DistanceMetric::Cosine)
        .expect("create collection");
    let collection = db.get_vector_collection("docs").expect("get collection");
    let rows = [
        (1, "tech", 2020),
        (2, "tech", 2021),
        (3, "tech", 2024),
        (4, "other", 2019),
        (5, "other", 2021),
        (6, "misc", 2022),
    ];
    let points: Vec<Point> = rows
        .iter()
        .map(|(id, category, year)| {
            Point::new(
                *id,
                vec![0.1, 0.2],
                Some(json!({ "category": category, "year": year })),
            )
        })
        .collect();
    collection.upsert(points).expect("upsert");
    (dir, db)
}

fn run_aggregate(db: &Database, sql: &str) -> Vec<Value> {
    let query = Parser::parse(sql).expect("parse");
    let result = db
        .execute_aggregate(&query, &HashMap::new())
        .expect("execute_aggregate");
    result.as_array().expect("array result").clone()
}

#[test]
fn group_by_limit_truncates_ordered_groups() {
    let (_dir, db) = open_grouped_db();
    let groups = run_aggregate(
        &db,
        "SELECT category, COUNT(*) AS n FROM docs GROUP BY category ORDER BY n DESC, category ASC LIMIT 1",
    );
    assert_eq!(
        groups,
        vec![json!({ "category": "tech", "n": 3 })],
        "LIMIT 1 must keep only the first group after ORDER BY"
    );
}

#[test]
fn group_by_offset_skips_leading_groups() {
    let (_dir, db) = open_grouped_db();
    let groups = run_aggregate(
        &db,
        "SELECT category, COUNT(*) AS n FROM docs GROUP BY category ORDER BY n DESC, category ASC LIMIT 1 OFFSET 1",
    );
    assert_eq!(
        groups,
        vec![json!({ "category": "other", "n": 2 })],
        "OFFSET must skip already-ordered groups before LIMIT applies"
    );
}

#[test]
fn group_by_offset_only_drops_prefix_without_capping() {
    let (_dir, db) = open_grouped_db();
    let groups = run_aggregate(
        &db,
        "SELECT category, COUNT(*) AS n FROM docs GROUP BY category ORDER BY n DESC, category ASC OFFSET 1",
    );
    assert_eq!(
        groups,
        vec![
            json!({ "category": "other", "n": 2 }),
            json!({ "category": "misc", "n": 1 })
        ],
        "OFFSET without LIMIT must return every remaining group"
    );
}

#[test]
fn group_by_without_limit_returns_every_group() {
    let (_dir, db) = open_grouped_db();
    let groups = run_aggregate(
        &db,
        "SELECT category, COUNT(*) AS n FROM docs GROUP BY category ORDER BY n DESC, category ASC",
    );
    assert_eq!(
        groups.len(),
        3,
        "a LIMIT-less GROUP BY must not be capped by any default SELECT limit"
    );
}
