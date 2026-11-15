//! BDD scenarios for `execute_match_rows`: MATCH rows shaped as core's
//! `MatchResult` (`bindings`, `depth`, `projected`), the contract of REST
//! `POST /collections/{name}/match` that the TS SDK's `matchQuery` returns.
//!
//! `executeQuery` returns one alias-keyed object per row and ignores the
//! RETURN projection; a client that wants core's shape had to re-derive it.

use serde_json::json;

use crate::database::DatabaseInner;
use crate::velesql_exec::execute;
use crate::velesql_match_core::execute_match_rows;

const ON_DOCS: Option<&str> = Some(r#"{"_collection": "docs"}"#);

/// `docs`: points 1, 2, 3 (`:Doc`, titled) and CITES edges 1 -> 2 (weight
/// 0.5) and 1 -> 3 (no properties).
fn docs(db: &mut DatabaseInner) {
    db.create_metadata_collection("docs")
        .expect("test: create docs");
    let store = db.get_shared_store("docs").expect("test: docs store");
    for id in 1..=3u64 {
        crate::store_insert::insert_with_payload(
            &mut store.borrow_mut(),
            id,
            &[],
            Some(json!({"_labels": ["Doc"], "title": format!("doc {id}")})),
        );
    }
    execute(
        db,
        "INSERT EDGE INTO docs (id = 10, source = 1, target = 2, label = 'CITES') WITH PROPERTIES (weight = 0.5)",
        None,
    )
    .expect("test: edge 10");
    execute(
        db,
        "INSERT EDGE INTO docs (id = 11, source = 1, target = 3, label = 'CITES')",
        None,
    )
    .expect("test: edge 11");
}

fn run(db: &mut DatabaseInner, sql: &str) -> Vec<serde_json::Value> {
    execute_match_rows(db, sql, ON_DOCS, None).expect("test: MATCH")
}

/// Rows sorted by the `b` binding, for order-independent comparison.
fn by_b(mut rows: Vec<serde_json::Value>) -> Vec<serde_json::Value> {
    rows.sort_by_key(|r| r["bindings"]["b"].as_u64());
    rows
}

#[test]
fn property_paths_project_under_their_expression_or_alias() {
    let mut db = DatabaseInner::new();
    docs(&mut db);

    let rows = by_b(run(
        &mut db,
        "MATCH (a:Doc)-[r:CITES]->(b:Doc) RETURN a.title, b.title AS cited, r.weight",
    ));
    assert_eq!(
        rows,
        vec![
            json!({
                "bindings": {"a": 1, "b": 2},
                "depth": 1,
                "projected": {"a.title": "doc 1", "cited": "doc 2", "r.weight": 0.5},
            }),
            // Edge 11 has no `weight`: the key is absent, as in core.
            json!({
                "bindings": {"a": 1, "b": 3},
                "depth": 1,
                "projected": {"a.title": "doc 1", "cited": "doc 3"},
            }),
        ]
    );
}

#[test]
fn a_bare_alias_projects_every_property_and_the_id() {
    let mut db = DatabaseInner::new();
    docs(&mut db);

    let rows = run(
        &mut db,
        "MATCH (a:Doc)-[:CITES]->(b:Doc) WHERE b.title = 'doc 2' RETURN b",
    );
    assert_eq!(
        rows,
        vec![json!({
            "bindings": {"a": 1, "b": 2},
            "depth": 1,
            "projected": {"b.title": "doc 2", "b._labels": ["Doc"], "b.id": 2},
        })]
    );
}

#[test]
fn a_single_node_row_has_depth_zero_and_reads_id() {
    let mut db = DatabaseInner::new();
    docs(&mut db);

    let rows = run(&mut db, "MATCH (n:Doc) WHERE n.title = 'doc 3' RETURN n.id");
    assert_eq!(
        rows,
        vec![json!({"bindings": {"n": 3}, "depth": 0, "projected": {"n.id": 3}})]
    );
}

#[test]
fn unnamed_nodes_are_not_bound() {
    let mut db = DatabaseInner::new();
    docs(&mut db);

    let rows = run(&mut db, "MATCH (a:Doc)-[:CITES]->(:Doc) RETURN a.title");
    assert_eq!(rows.len(), 2);
    for row in rows {
        assert_eq!(row["bindings"], json!({"a": 1}));
    }
}

#[test]
fn order_by_and_limit_apply_before_shaping() {
    let mut db = DatabaseInner::new();
    docs(&mut db);

    let rows = run(
        &mut db,
        "MATCH (a:Doc)-[:CITES]->(b:Doc) RETURN b.title ORDER BY b.title DESC LIMIT 1",
    );
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["projected"]["b.title"], "doc 3");
}

#[test]
fn a_statement_that_is_not_a_match_is_refused() {
    let mut db = DatabaseInner::new();
    docs(&mut db);

    let err = execute_match_rows(&mut db, "SELECT * FROM docs LIMIT 1", None, None)
        .expect_err("not a MATCH");
    assert!(err.contains("MATCH"), "{err}");
}
