#![cfg(all(test, feature = "persistence"))]
//! EPIC-081 phase 3a — scalar `ORDER BY <field>` index advisor.
//!
//! The advisor records eligible `ORDER BY <field>` queries that fall back to
//! the exhaustive sort for want of a *fully covering* secondary index, and
//! **never alters a query result**. These tests pin: recording on the
//! eligible-but-unindexed shape (`Missing`), the `BuiltButUncovered` state,
//! behaviour-neutrality, shape-isolation (only `ORDER BY` shapes observed), and
//! the `min_observations` threshold.

use serde_json::json;
use std::collections::HashMap;
use tempfile::TempDir;
use velesdb_core::{DistanceMetric, OrderByIndexState, Point, StorageMode, VectorCollection};

fn build(rows: &[(u64, i64)]) -> (VectorCollection, TempDir) {
    let dir = TempDir::new().expect("temp dir");
    let collection = VectorCollection::create(
        dir.path().join("docs"),
        "docs",
        2,
        DistanceMetric::Cosine,
        StorageMode::Full,
    )
    .expect("create collection");
    let points: Vec<Point> = rows
        .iter()
        .map(|&(id, year)| Point::new(id, vec![1.0, 0.0], Some(json!({ "year": year }))))
        .collect();
    collection.upsert(points).expect("upsert");
    (collection, dir)
}

const ROWS: &[(u64, i64)] = &[(1, 2020), (2, 2022), (3, 2021), (4, 2023), (5, 2019)];

fn run(c: &VectorCollection, sql: &str) -> Vec<u64> {
    c.execute_query_str(sql, &HashMap::new())
        .expect("query")
        .iter()
        .map(|r| r.point.id)
        .collect()
}

#[test]
fn records_missing_index_for_eligible_order_by() {
    let (c, _d) = build(ROWS);
    // No index on `year`: the eligible ORDER BY falls back and is recorded.
    let _ = run(&c, "SELECT * FROM docs ORDER BY year DESC LIMIT 3");
    let advice = c.order_by_index_advice(1);
    assert_eq!(advice.len(), 1);
    assert_eq!(advice[0].field, "year");
    assert_eq!(advice[0].observed_count, 1);
    assert_eq!(advice[0].state, OrderByIndexState::Missing);
}

#[test]
fn recording_is_behaviour_neutral() {
    let (c, _d) = build(ROWS);
    let ids = run(&c, "SELECT * FROM docs ORDER BY year DESC LIMIT 3");
    // The advisor did not change the result: DESC by year → 2023, 2022, 2021.
    assert_eq!(ids, vec![4, 2, 3]);
    // ...and it still recorded the observation.
    assert_eq!(c.order_by_index_advice(1).len(), 1);
}

#[test]
fn covered_fast_path_records_nothing() {
    let (c, _d) = build(ROWS);
    c.create_index("year").expect("create_index");
    let _ = run(&c, "SELECT * FROM docs ORDER BY year DESC LIMIT 3");
    // The fast path served it → no fall-back, no advice.
    assert!(c.order_by_index_advice(1).is_empty());
}

#[test]
fn non_order_by_query_is_not_recorded() {
    let (c, _d) = build(ROWS);
    // No ORDER BY → the ordered-index route is irrelevant and never observed,
    // now or after later phases broaden eligibility.
    let _ = run(&c, "SELECT * FROM docs LIMIT 3");
    assert!(c.order_by_index_advice(1).is_empty());
}

#[test]
fn where_clause_currently_not_recorded() {
    let (c, _d) = build(ROWS);
    // A WHERE clause disqualifies the phase-2 route before the advisor hook, so
    // it is not observed today. (EPIC-081 phase 3b — WHERE-filtered top-k —
    // will make this shape eligible; update this expectation then.)
    let _ = run(
        &c,
        "SELECT * FROM docs WHERE year >= 2021 ORDER BY year DESC LIMIT 2",
    );
    assert!(c.order_by_index_advice(1).is_empty());
}

#[test]
fn built_but_uncovered_index_is_surfaced() {
    let (c, _d) = build(&[(1, 2020), (2, 2022)]);
    c.create_index("year").expect("create_index");
    // Insert a row missing `year` → coverage breaks → the fast path declines.
    c.upsert(vec![Point::new(
        3,
        vec![1.0, 0.0],
        Some(json!({ "tag": "x" })),
    )])
    .expect("upsert");
    let _ = run(&c, "SELECT * FROM docs ORDER BY year DESC LIMIT 5");
    let advice = c.order_by_index_advice(1);
    assert_eq!(advice.len(), 1);
    assert_eq!(advice[0].field, "year");
    assert_eq!(advice[0].state, OrderByIndexState::BuiltButUncovered);
}

#[test]
fn resolved_field_drops_out_of_advice() {
    let (c, _d) = build(ROWS);
    // Observe a fall-back (no index → Missing).
    let _ = run(&c, "SELECT * FROM docs ORDER BY year DESC LIMIT 3");
    assert_eq!(c.order_by_index_advice(1).len(), 1);
    // Create a fully-covering index: the field is now resolved (the fast path
    // fires), so the advice — derived from live coverage — drops it.
    c.create_index("year").expect("create_index");
    assert!(
        c.order_by_index_advice(1).is_empty(),
        "a now-covering index resolves the advice"
    );
}

#[test]
fn min_observations_threshold_filters() {
    let (c, _d) = build(ROWS);
    let _ = run(&c, "SELECT * FROM docs ORDER BY year DESC LIMIT 3");
    let _ = run(&c, "SELECT * FROM docs ORDER BY year ASC LIMIT 3");
    // Two eligible fall-backs on `year`.
    assert_eq!(c.order_by_index_advice(2)[0].observed_count, 2);
    assert!(
        c.order_by_index_advice(3).is_empty(),
        "below threshold → no advice"
    );
}
