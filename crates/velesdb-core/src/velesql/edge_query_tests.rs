use super::EdgeQuery;
use crate::velesql::{Parser, Query};

fn where_of(sql: &str) -> Option<crate::velesql::Condition> {
    let Query { dml, .. } = Parser::parse(sql).expect("test: parse");
    match dml {
        Some(crate::velesql::DmlStatement::SelectEdges(stmt)) => stmt.where_clause,
        other => panic!("test: expected SELECT EDGES, got {other:?}"),
    }
}

fn query(sql: &str) -> crate::error::Result<EdgeQuery> {
    EdgeQuery::from_where(where_of(sql).as_ref())
}

#[test]
fn an_and_chain_of_the_three_columns_is_read() {
    let q = query("SELECT EDGES FROM g WHERE label = 'KNOWS' AND (source = 1) AND target = 2")
        .expect("supported");
    assert_eq!(
        (q.source, q.target, q.label.as_deref()),
        (Some(1), Some(2), Some("KNOWS"))
    );
    assert!(q.matches(1, 2, "KNOWS"));
    assert!(!q.matches(1, 3, "KNOWS"));
}

#[test]
fn no_where_matches_every_edge() {
    let q = query("SELECT EDGES FROM g").expect("supported");
    assert!(q.matches(7, 8, "anything"));
}

#[test]
fn a_column_pinned_twice_to_different_values_matches_nothing() {
    let q = query("SELECT EDGES FROM g WHERE source = 1 AND source = 2").expect("supported");
    assert!(!q.matches(1, 9, "x") && !q.matches(2, 9, "x"));
}

/// An unsupported conjunct is refused, never ignored: ignoring it would
/// return edges the query excluded.
#[test]
fn every_shape_outside_the_grammar_is_refused() {
    for sql in [
        "SELECT EDGES FROM g WHERE source = 1 OR source = 2",
        "SELECT EDGES FROM g WHERE label != 'X'",
        "SELECT EDGES FROM g WHERE weight = 5",
        "SELECT EDGES FROM g WHERE source = $s",
        "SELECT EDGES FROM g WHERE source = -1",
        "SELECT EDGES FROM g WHERE label = 3",
    ] {
        let err = query(sql).expect_err(sql);
        assert_eq!(err.code(), "VELES-010", "{sql}: {err}");
    }
}

#[test]
fn the_row_nests_properties_under_their_own_key() {
    let row = EdgeQuery::row(5, 1, 2, "KNOWS", &serde_json::json!({"source": "x"}));
    assert_eq!(row["edge_id"], 5);
    assert_eq!(
        row["source"], 1,
        "a property named source cannot shadow the column"
    );
    assert_eq!(row["properties"]["source"], "x");
}
