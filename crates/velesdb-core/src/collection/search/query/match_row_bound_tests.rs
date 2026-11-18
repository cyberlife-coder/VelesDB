//! The MATCH walk's row bound: an `ORDER BY` (or a `/match` score sort)
//! ranks every row the pattern produces, never a set the walk cut short.
//!
//! The walk stopped at the shared `MAX_LIMIT` ceiling even under an
//! `ORDER BY`, and that ceiling equals `DEFAULT_MAX_CARDINALITY`, so the
//! cardinality check could not fire before the stop: a pattern producing more
//! rows sorted the first 100 000 and returned the wrong top rows. The walk now
//! holds at most the cardinality bound and refuses the row past it with the
//! error the WASM walk raises for the same pattern; it stops early only at a
//! `LIMIT` nothing reorders.

use crate::collection::graph::GraphEdge;
use crate::collection::types::Collection;
use crate::error::Error;
use crate::guardrails::{GuardRailViolation, GuardRails, QueryLimits, DEFAULT_MAX_CARDINALITY};
use crate::point::Point;
use crate::velesql::{MatchClause, Parser};
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::TempDir;

/// 317² = 100 489 rows: past `DEFAULT_MAX_CARDINALITY` and `MAX_LIMIT`.
const OVER_BOUND_FAN: u64 = 317;

const BOW_TIE: &str = "MATCH (a)-[:R]->(h)-[:R]->(c) RETURN a.rank, c.rank";

/// `fan` nodes `a` (ids `1..=fan`) into hub 0, the hub out to `fan` nodes
/// `c` (ids `fan+1..=2*fan`): `(a)-[:R]->(h)-[:R]->(c)` matches `fan²` rows.
/// Every node is a point whose `rank` is its id, its vector `[id, 1]`.
fn bow_tie(dir: &TempDir, fan: u64) -> Collection {
    let col = Collection::create(
        dir.path().join("bow_tie"),
        2,
        crate::distance::DistanceMetric::Euclidean,
    )
    .expect("test: create collection");
    #[allow(clippy::cast_precision_loss)]
    let points: Vec<Point> = (0..=2 * fan)
        .map(|id| {
            Point::new(
                id,
                vec![id as f32, 1.0],
                Some(serde_json::json!({ "rank": id })),
            )
        })
        .collect();
    col.upsert(points).expect("test: upsert");
    for i in 1..=fan {
        col.add_edge(GraphEdge::new(2 * i, i, 0, "R").expect("test: edge"))
            .expect("test: add edge");
        col.add_edge(GraphEdge::new(2 * i + 1, 0, fan + i, "R").expect("test: edge"))
            .expect("test: add edge");
    }
    col
}

fn with_max_cardinality(col: &mut Collection, max_cardinality: usize) {
    col.runtime.guard_rails = Arc::new(GuardRails::with_limits(QueryLimits {
        max_cardinality,
        ..QueryLimits::default()
    }));
}

fn clause(sql: &str) -> MatchClause {
    Parser::parse(sql)
        .expect("test: parse")
        .match_clause
        .expect("test: a MATCH statement")
}

fn cardinality_error(max: usize) -> String {
    Error::from(GuardRailViolation::CardinalityExceeded {
        max,
        actual: max + 1,
    })
    .to_string()
}

fn ends(rows: &[super::match_exec::MatchResult]) -> Vec<(u64, u64)> {
    rows.iter()
        .map(|row| (row.bindings["a"], row.bindings["c"]))
        .collect()
}

/// Under a bound above `MAX_LIMIT`, an `ORDER BY` over 100 489 rows ranks
/// all of them, from the very top row to the very bottom one.
#[test]
fn order_by_ranks_every_row_past_max_limit() {
    let dir = TempDir::new().unwrap();
    let mut col = bow_tie(&dir, OVER_BOUND_FAN);
    with_max_cardinality(&mut col, 200_000);
    let ordered = clause(&format!("{BOW_TIE} ORDER BY a.rank DESC, c.rank DESC"));
    let fan = OVER_BOUND_FAN;

    let rows = col
        .match_query_ordered(&ordered, &HashMap::new())
        .expect("test: within the bound");

    let ranked = ends(&rows);
    assert_eq!(ranked.len() as u64, fan * fan, "every row is ranked");
    assert_eq!(ranked[0], (fan, 2 * fan));
    assert_eq!(ranked[ranked.len() - 1], (1, fan + 1));
}

/// Under the default bound the same `ORDER BY` is refused at the first row
/// past it, with the count the WASM walk reports, not a doubled one.
#[test]
fn order_by_past_the_default_bound_is_refused_at_the_first_row_past_it() {
    let dir = TempDir::new().unwrap();
    let col = bow_tie(&dir, OVER_BOUND_FAN);
    let ordered = clause(&format!("{BOW_TIE} ORDER BY c.rank DESC LIMIT 3"));

    let err = col
        .match_query_ordered(&ordered, &HashMap::new())
        .expect_err("an ORDER BY past the bound must be refused");

    assert_eq!(err.to_string(), cardinality_error(DEFAULT_MAX_CARDINALITY));
}

/// The guard-rail-free entry point refuses too, instead of sorting the
/// first `MAX_LIMIT` rows.
#[test]
fn execute_match_refuses_an_order_by_past_the_bound() {
    let dir = TempDir::new().unwrap();
    let col = bow_tie(&dir, OVER_BOUND_FAN);
    let ordered = clause(&format!("{BOW_TIE} ORDER BY c.rank DESC LIMIT 3"));

    let err = col
        .execute_match(&ordered, &HashMap::new())
        .expect_err("an ORDER BY past the bound must be refused");

    assert_eq!(err.to_string(), cardinality_error(DEFAULT_MAX_CARDINALITY));
}

/// Nothing reorders the rows, so the walk still stops at `LIMIT`, well
/// before the bound.
#[test]
fn limit_without_order_by_still_stops_the_walk_early() {
    let dir = TempDir::new().unwrap();
    let col = bow_tie(&dir, OVER_BOUND_FAN);

    let rows = col
        .match_query_ordered(&clause(&format!("{BOW_TIE} LIMIT 3")), &HashMap::new())
        .expect("test: limited walk");

    assert_eq!(rows.len(), 3);
}

/// A single-node pattern holds its rows under the same bound: the row past
/// it is refused during the walk.
#[test]
fn a_single_node_walk_is_refused_at_the_first_row_past_the_bound() {
    let dir = TempDir::new().unwrap();
    let mut col = bow_tie(&dir, 4);
    with_max_cardinality(&mut col, 5);
    let ordered = clause("MATCH (n) RETURN n.rank ORDER BY n.rank DESC LIMIT 1");

    let err = col
        .match_query_ordered(&ordered, &HashMap::new())
        .expect_err("9 nodes past a bound of 5 must be refused");

    assert_eq!(err.to_string(), cardinality_error(5));
}

/// Rows within the bound are counted once: 100 rows over 120 traversed
/// edges pass a bound of 150 (the walk's periodic report plus the final
/// check used to count them twice).
#[test]
fn rows_within_the_bound_are_counted_once() {
    let dir = TempDir::new().unwrap();
    let mut col = bow_tie(&dir, 10);
    with_max_cardinality(&mut col, 150);

    let rows = col
        .match_query_ordered(&clause(BOW_TIE), &HashMap::new())
        .expect("100 rows are within a bound of 150");

    assert_eq!(rows.len(), 100);
}

/// REST `/match`'s query vector sorts the rows by score before `LIMIT`,
/// so the walk reads every row: the best-scoring `c` is returned whichever
/// row the walk reached it on.
#[test]
fn a_scored_match_reads_every_row_before_its_limit() {
    let dir = TempDir::new().unwrap();
    let fan = 5;
    let col = bow_tie(&dir, fan);
    let limited = clause("MATCH (a)-[:R]->(h)-[:R]->(c) WHERE a.rank = 1 RETURN c LIMIT 1");

    for target in fan + 1..=2 * fan {
        #[allow(clippy::cast_precision_loss)]
        let query = [target as f32, 1.0];
        let rows = col
            .execute_match_with_similarity(&limited, &query, f32::MAX, &HashMap::new())
            .expect("test: scored match");

        assert_eq!(rows.len(), 1, "target {target}");
        assert_eq!(rows[0].node_id, target, "the nearest c must rank first");
    }
}
