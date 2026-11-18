//! The MATCH walk's row bound and its LIMIT early stop.
//!
//! The walk collected every row before ORDER BY / LIMIT, with no bound: a
//! fan-out pattern grew one `Vec` until the tab ran out of memory. It now
//! refuses the row past core's `DEFAULT_MAX_CARDINALITY` with core's own
//! `VELES-027` error, and — when nothing orders the rows — stops at `LIMIT`
//! rows, which are the rows the truncation kept before.

use velesdb_core::guardrails::{GuardRailViolation, DEFAULT_MAX_CARDINALITY};
use velesdb_core::velesql::{InsertEdgeStatement, InsertNodeStatement, Parser};

use crate::database::DatabaseInner;
use crate::velesql_graph::execute_match;
use crate::velesql_value::Params;

fn node(db: &mut DatabaseInner, id: u64, label: &str) {
    let stmt = InsertNodeStatement {
        collection: "graph".to_string(),
        node_id: id,
        payload: serde_json::json!({"name": format!("n{id}"), "labels": [label]}),
    };
    crate::velesql_graph::insert_node(db, &stmt).expect("test: insert node");
}

fn edge(db: &mut DatabaseInner, source: u64, target: u64) {
    let stmt = InsertEdgeStatement {
        collection: "graph".to_string(),
        edge_id: None,
        source,
        target,
        label: "R".to_string(),
        properties: Vec::new(),
    };
    crate::velesql_graph::insert_edge(db, &stmt, &Params::new()).expect("test: insert edge");
}

/// `fan` `A` nodes into one hub `H` (id 0), the hub out to `fan` `C` nodes:
/// `(a:A)-[:R]->(h:H)-[:R]->(c:C)` matches `fan * fan` rows over `2 * fan`
/// edges.
fn bow_tie(fan: u64) -> DatabaseInner {
    let mut db = DatabaseInner::new();
    node(&mut db, 0, "H");
    for i in 1..=fan {
        node(&mut db, i, "A");
        node(&mut db, fan + i, "C");
        edge(&mut db, i, 0);
        edge(&mut db, 0, fan + i);
    }
    db
}

const BOW_TIE: &str = "MATCH (a:A)-[:R]->(h:H)-[:R]->(c:C) RETURN a, c";

fn run(db: &mut DatabaseInner, sql: &str) -> Result<Vec<serde_json::Value>, String> {
    let query = Parser::parse(sql).expect("test: parse");
    execute_match(db, &query, &Params::new()).map(|rows| {
        rows.iter()
            .map(|row| serde_json::from_str(row.data_json_ref()).expect("test: row json"))
            .collect()
    })
}

/// 317² = 100 489 rows: past the bound, so the walk is refused with the
/// error core raises for the same violation, word for word.
#[test]
fn a_walk_past_the_cardinality_bound_fails_with_cores_error() {
    let fan = 317;
    assert!(fan * fan > DEFAULT_MAX_CARDINALITY as u64, "test: premise");
    let mut db = bow_tie(fan);

    let err = run(&mut db, BOW_TIE).expect_err("an unbounded walk must be refused");

    let core = velesdb_core::Error::from(GuardRailViolation::CardinalityExceeded {
        max: DEFAULT_MAX_CARDINALITY,
        actual: DEFAULT_MAX_CARDINALITY + 1,
    });
    assert_eq!(err, core.to_string());
    assert!(err.contains("VELES-027"), "{err}");
}

/// ORDER BY needs every row before it can keep the best ones, so the
/// bound applies even under a small LIMIT.
#[test]
fn order_by_reads_every_row_so_the_bound_still_applies() {
    let mut db = bow_tie(317);

    let err = run(&mut db, &format!("{BOW_TIE} ORDER BY c.name LIMIT 3"))
        .expect_err("ORDER BY must read the whole walk");

    assert!(err.contains("VELES-027"), "{err}");
}

/// Without ORDER BY the walk stops at LIMIT rows, so the same pattern that
/// is refused unlimited answers under a LIMIT, as in core.
#[test]
fn limit_without_order_by_stops_the_walk_before_the_bound() {
    let mut db = bow_tie(317);

    let found = run(&mut db, &format!("{BOW_TIE} LIMIT 3")).expect("test: limited walk");

    assert_eq!(found.len(), 3);
}

/// The early stop keeps exactly the rows the truncation kept: the first
/// LIMIT rows of the walk, in walk order.
#[test]
fn limit_without_order_by_returns_the_first_rows_of_the_full_walk() {
    let mut db = bow_tie(4);

    let all = run(&mut db, BOW_TIE).expect("test: full walk");
    let limited = run(&mut db, &format!("{BOW_TIE} LIMIT 5")).expect("test: limited walk");

    assert_eq!(all.len(), 16);
    assert_eq!(limited, all[..5].to_vec());
}
