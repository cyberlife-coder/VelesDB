//! `vector NEAR` under `OR`: the disjunction keeps the NEAR ranking.
//!
//! In a boolean context a `NEAR` leaf is TRUE for every candidate — the WHERE
//! evaluator (`where_eval.rs`, `Condition::VectorSearch(_) => Ok(Some(true))`)
//! and the planner's selectivity estimate (`strip_vector_predicates`, which
//! reads an `OR` with a vector side as selectivity 1.0) both say so — and the
//! NEAR vector ranks the rows. So `NEAR $v OR <metadata>` returns the rows the
//! NEAR search ranks, carrying their NEAR scores, and the rest of the WHERE
//! applies with the NEAR leaf read as TRUE.
//!
//! The defect these pin: the shape used to be routed to the `similarity()`
//! union executor, which only knows `similarity()` thresholds. Its vector leg
//! found no threshold and returned nothing, so every NEAR hit was dropped and
//! only the metadata side came back, each row scored 1.0.

#![cfg(all(test, feature = "persistence"))]

use std::collections::HashMap;

use crate::collection::Collection;
use crate::point::SearchResult;
use crate::test_fixtures::fixtures::{make_point_with_payload, setup_collection};
use crate::velesql::Parser;

/// The executor conformance dataset (`conformance/velesql_executor_cases.json`).
fn docs() -> (tempfile::TempDir, Collection) {
    let (dir, col) = setup_collection(4);
    let row = |id, vector: [f32; 4], category: &str, year: i64| {
        make_point_with_payload(
            id,
            vector.to_vec(),
            serde_json::json!({ "category": category, "year": year }),
        )
    };
    col.upsert(vec![
        row(1, [1.0, 0.0, 0.0, 0.0], "tech", 2020),
        row(2, [0.0, 1.0, 0.0, 0.0], "tech", 2022),
        row(3, [0.0, 0.0, 1.0, 0.0], "other", 2021),
        row(4, [0.0, 0.0, 0.0, 1.0], "tech", 2023),
        row(5, [0.5, 0.5, 0.0, 0.0], "other", 2019),
    ])
    .expect("test: upsert");
    (dir, col)
}

/// Cosine to `[0.8, 0.4, 0.2, 0.1]` ranks every row apart: 5, 1, 2, 3, 4.
fn params() -> HashMap<String, serde_json::Value> {
    HashMap::from([("v".to_string(), serde_json::json!([0.8, 0.4, 0.2, 0.1]))])
}

fn run(col: &Collection, sql: &str) -> Vec<SearchResult> {
    let query = Parser::parse(sql).unwrap_or_else(|e| panic!("parse {sql}: {e}"));
    col.execute_query(&query, &params())
        .unwrap_or_else(|e| panic!("execute {sql}: {e}"))
}

fn ids(results: &[SearchResult]) -> Vec<u64> {
    results.iter().map(|r| r.point.id).collect()
}

fn assert_same_rows_and_scores(actual: &[SearchResult], expected: &[SearchResult]) {
    assert_eq!(ids(actual), ids(expected));
    for (a, e) in actual.iter().zip(expected) {
        assert!(
            (a.score - e.score).abs() < 1e-6,
            "id {}: score {} is not the NEAR score {}",
            a.point.id,
            a.score,
            e.score
        );
    }
}

#[test]
fn near_or_metadata_returns_the_near_ranking_with_near_scores() {
    let (_dir, col) = docs();
    let near = run(&col, "SELECT * FROM docs WHERE vector NEAR $v LIMIT 5");
    assert_eq!(ids(&near), [5, 1, 2, 3, 4], "precondition: distinct ranks");

    let disjunction = run(
        &col,
        "SELECT * FROM docs WHERE vector NEAR $v OR category = 'other' LIMIT 5",
    );

    assert_same_rows_and_scores(&disjunction, &near);
}

#[test]
fn the_near_leaf_may_sit_on_either_side_of_the_or() {
    let (_dir, col) = docs();
    let near = run(&col, "SELECT * FROM docs WHERE vector NEAR $v LIMIT 3");

    let disjunction = run(
        &col,
        "SELECT * FROM docs WHERE category = 'other' OR vector NEAR $v LIMIT 3",
    );

    assert_same_rows_and_scores(&disjunction, &near);
}

/// The rest of the WHERE still filters: `(NEAR OR x) AND y` reads as
/// `TRUE AND y`, so only `y` restricts the NEAR ranking.
#[test]
fn an_outer_conjunct_still_filters_the_disjunction() {
    let (_dir, col) = docs();
    let filtered_near = run(
        &col,
        "SELECT * FROM docs WHERE vector NEAR $v AND year >= 2021 LIMIT 5",
    );
    assert_eq!(ids(&filtered_near), [2, 3, 4], "precondition");

    let disjunction = run(
        &col,
        "SELECT * FROM docs WHERE (vector NEAR $v OR category = 'other') AND year >= 2021 \
         LIMIT 5",
    );

    assert_same_rows_and_scores(&disjunction, &filtered_near);
}

/// A scalar ORDER BY fetches exhaustively before sorting, so the disjunction
/// returns every row the NEAR leaf admits — all of them — in `year` order.
#[test]
fn a_scalar_order_by_sorts_the_whole_disjunction() {
    let (_dir, col) = docs();

    let ordered = run(
        &col,
        "SELECT * FROM docs WHERE vector NEAR $v OR category = 'other' ORDER BY year ASC LIMIT 5",
    );

    assert_eq!(ids(&ordered), [5, 1, 3, 2, 4]);
}

/// The `similarity()` union is untouched: its metadata side still contributes
/// rows the threshold rejects (#2403).
#[test]
fn a_similarity_disjunction_keeps_the_union_executor() {
    let (_dir, col) = docs();

    let mut union = ids(&run(
        &col,
        "SELECT * FROM docs WHERE similarity(vector, $v) > 0.9 OR category = 'other' LIMIT 5",
    ));
    union.sort_unstable();

    assert_eq!(union, [3, 5]);
}
