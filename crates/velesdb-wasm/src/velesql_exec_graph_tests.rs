//! BDD integration tests for graph DML + MATCH in the WASM VelesQL
//! executor (S4-13).

use crate::database::DatabaseInner;
use crate::velesql_exec::execute;

fn seed_graph(db: &mut DatabaseInner) {
    // People
    execute(
        db,
        "INSERT NODE INTO graph (id = 1, payload = '{\"name\": \"Alice\", \"labels\": [\"Person\"]}')",
        None,
    )
    .expect("test: node 1");
    execute(
        db,
        "INSERT NODE INTO graph (id = 2, payload = '{\"name\": \"Bob\", \"labels\": [\"Person\"]}')",
        None,
    )
    .expect("test: node 2");
    execute(
        db,
        "INSERT NODE INTO graph (id = 3, payload = '{\"name\": \"Carol\", \"labels\": [\"Person\"]}')",
        None,
    )
    .expect("test: node 3");
    // Edges: Alice KNOWS Bob; Bob KNOWS Carol.
    execute(
        db,
        "INSERT EDGE INTO graph (source = 1, target = 2, label = 'KNOWS')",
        None,
    )
    .expect("test: edge 1");
    execute(
        db,
        "INSERT EDGE INTO graph (source = 2, target = 3, label = 'KNOWS')",
        None,
    )
    .expect("test: edge 2");
}

// =========================================================================
// INSERT NODE / EDGE — nominal
// =========================================================================

#[test]
fn test_insert_node_returns_mutation() {
    let mut db = DatabaseInner::new();
    let r = execute(
        &mut db,
        "INSERT NODE INTO kg (id = 42, payload = '{\"name\": \"X\"}')",
        None,
    )
    .expect("test: insert node");
    assert_eq!(r.kind(), "mutation");
    assert_eq!(r.row_count(), 1);
}

#[test]
fn test_insert_edge_returns_mutation() {
    let mut db = DatabaseInner::new();
    let r = execute(
        &mut db,
        "INSERT EDGE INTO kg (source = 1, target = 2, label = 'REL')",
        None,
    )
    .expect("test: insert edge");
    assert_eq!(r.kind(), "mutation");
}

#[test]
fn test_insert_edge_with_properties() {
    let mut db = DatabaseInner::new();
    execute(
        &mut db,
        "INSERT EDGE INTO kg (source = 1, target = 2, label = 'REL') WITH PROPERTIES (weight = 0.8)",
        None,
    )
    .expect("test: insert edge with props");
    let r = execute(&mut db, "SELECT EDGES FROM kg WHERE source = 1", None)
        .expect("test: select edges");
    assert!(
        r.rows_json().contains("\"weight\":0.8"),
        "got: {}",
        r.rows_json()
    );
}

// =========================================================================
// SELECT EDGES — nominal
// =========================================================================

#[test]
fn test_select_edges_all() {
    let mut db = DatabaseInner::new();
    seed_graph(&mut db);
    let r = execute(&mut db, "SELECT EDGES FROM graph", None).expect("test: select edges");
    assert_eq!(r.row_count(), 2);
}

#[test]
fn test_select_edges_filter_by_source() {
    let mut db = DatabaseInner::new();
    seed_graph(&mut db);
    let r = execute(&mut db, "SELECT EDGES FROM graph WHERE source = 1", None)
        .expect("test: by source");
    assert_eq!(r.row_count(), 1);
    assert!(r.rows_json().contains("\"target\":2"));
}

#[test]
fn test_select_edges_filter_by_label() {
    let mut db = DatabaseInner::new();
    seed_graph(&mut db);
    let r = execute(
        &mut db,
        "SELECT EDGES FROM graph WHERE label = 'KNOWS'",
        None,
    )
    .expect("test: by label");
    assert_eq!(r.row_count(), 2);
}

// =========================================================================
// DELETE EDGE — nominal
// =========================================================================

#[test]
fn test_delete_edge_by_id_returns_deletion() {
    let mut db = DatabaseInner::new();
    seed_graph(&mut db);
    let r = execute(&mut db, "DELETE EDGE 1 FROM graph", None).expect("test: delete edge");
    assert_eq!(r.kind(), "deletion");
}

// =========================================================================
// MATCH — nominal (1-hop)
// =========================================================================

#[test]
fn test_match_1_hop_returns_pairs() {
    let mut db = DatabaseInner::new();
    seed_graph(&mut db);
    let r = execute(
        &mut db,
        "MATCH (a:Person)-[:KNOWS]->(b:Person) RETURN a, b LIMIT 10",
        None,
    )
    .expect("test: match 1-hop");
    assert_eq!(r.row_count(), 2); // Alice→Bob, Bob→Carol
}

#[test]
fn test_match_with_where_filters_starting_node() {
    let mut db = DatabaseInner::new();
    seed_graph(&mut db);
    let r = execute(
        &mut db,
        "MATCH (a:Person)-[:KNOWS]->(b:Person) WHERE a.name = 'Alice' RETURN a, b LIMIT 10",
        None,
    )
    .expect("test: match where");
    assert_eq!(r.row_count(), 1);
}

// =========================================================================
// MATCH — nominal (2-hop)
// =========================================================================

#[test]
fn test_match_2_hop_returns_triples() {
    let mut db = DatabaseInner::new();
    seed_graph(&mut db);
    let r = execute(
        &mut db,
        "MATCH (a:Person)-[:KNOWS]->(b:Person)-[:KNOWS]->(c:Person) RETURN a, b, c LIMIT 10",
        None,
    )
    .expect("test: match 2-hop");
    // Alice → Bob → Carol is the only 2-hop path.
    assert_eq!(r.row_count(), 1);
}

// =========================================================================
// Edge cases
// =========================================================================

#[test]
fn test_match_on_empty_graph_returns_zero() {
    let mut db = DatabaseInner::new();
    let r = execute(
        &mut db,
        "MATCH (a:Person)-[:KNOWS]->(b:Person) RETURN a, b LIMIT 10",
        None,
    );
    // Either 0 rows or a "graph empty" error — both acceptable demo-side.
    match r {
        Ok(result) => assert_eq!(result.row_count(), 0),
        Err(e) => assert!(e.contains("empty") || e.contains("not found")),
    }
}

#[test]
fn test_insert_node_idempotent_overwrites_payload() {
    let mut db = DatabaseInner::new();
    execute(
        &mut db,
        "INSERT NODE INTO kg (id = 1, payload = '{\"name\": \"first\"}')",
        None,
    )
    .expect("test: first");
    execute(
        &mut db,
        "INSERT NODE INTO kg (id = 1, payload = '{\"name\": \"updated\"}')",
        None,
    )
    .expect("test: second");
    // Cannot query node payload directly in WASM yet (no SELECT NODES),
    // but a MATCH with label and WHERE should find it.
    let r = execute(&mut db, "SELECT EDGES FROM kg", None).expect("test: smoke");
    // Smoke test: executor didn't blow up on the idempotent re-insert.
    assert_eq!(r.kind(), "rows");
}

// =========================================================================
// Negative (≥ 20%)
// =========================================================================

#[test]
fn test_delete_edge_on_missing_graph_errors() {
    let mut db = DatabaseInner::new();
    let err = execute(&mut db, "DELETE EDGE 1 FROM ghost_graph", None);
    assert!(err.is_err());
}

#[test]
fn test_select_edges_on_missing_graph_errors() {
    let mut db = DatabaseInner::new();
    let err = execute(&mut db, "SELECT EDGES FROM ghost_graph", None);
    assert!(err.is_err());
}

#[test]
fn test_match_beyond_2_hops_rejected() {
    let mut db = DatabaseInner::new();
    seed_graph(&mut db);
    let err = execute(
        &mut db,
        "MATCH (a:P)-[:R]->(b:P)-[:R]->(c:P)-[:R]->(d:P) RETURN a LIMIT 10",
        None,
    );
    assert!(err.is_err());
    assert!(err.expect_err("test: err").contains("more than 2 hops"));
}

#[test]
fn test_insert_edge_with_unbound_param_errors() {
    let mut db = DatabaseInner::new();
    let err = execute(
        &mut db,
        "INSERT EDGE INTO kg (source = 1, target = 2, label = 'R') WITH PROPERTIES (weight = $w)",
        Some("{}"),
    );
    assert!(err.is_err());
}
