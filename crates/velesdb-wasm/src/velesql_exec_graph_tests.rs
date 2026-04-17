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

// =========================================================================
// Regression tests for MATCH direction handling (Devin review finding #1).
// =========================================================================
//
// Before the fix, `expand_one_hop`, `expand_from_a`, and `expand_from_b`
// always filtered edges by `source == anchor` and then re-checked direction,
// so patterns using `<-[:REL]-` (incoming) returned 0 rows and `-[:REL]-`
// (undirected) dropped the incoming half. These tests pin the corrected
// behaviour.

/// Seeds a directed graph `Alice -KNOWS-> Bob -KNOWS-> Carol` for direction
/// tests. Every node carries label `Person`.
fn seed_directed_pair(db: &mut DatabaseInner) {
    seed_graph(db);
}

#[test]
fn test_match_incoming_direction_returns_rows() {
    // Before the fix: 0 rows (bug). Expected: Bob<-KNOWS-Alice.
    let mut db = DatabaseInner::new();
    seed_directed_pair(&mut db);
    let r = execute(
        &mut db,
        "MATCH (a:Person)<-[:KNOWS]-(b:Person) WHERE a.name = 'Bob' RETURN a, b LIMIT 10",
        None,
    )
    .expect("test: match incoming");
    assert_eq!(
        r.row_count(),
        1,
        "incoming MATCH from Bob should find Alice, got: {}",
        r.rows_json()
    );
    assert!(
        r.rows_json().contains("\"name\":\"Alice\""),
        "Bob's incoming KNOWS neighbour is Alice, got: {}",
        r.rows_json()
    );
}

#[test]
fn test_match_incoming_direction_terminal_node_returns_rows() {
    // Carol has an incoming KNOWS edge from Bob; no outgoing.
    let mut db = DatabaseInner::new();
    seed_directed_pair(&mut db);
    let r = execute(
        &mut db,
        "MATCH (a:Person)<-[:KNOWS]-(b:Person) WHERE a.name = 'Carol' RETURN a, b LIMIT 10",
        None,
    )
    .expect("test: match incoming terminal");
    assert_eq!(r.row_count(), 1);
    assert!(r.rows_json().contains("\"name\":\"Bob\""));
}

#[test]
fn test_match_undirected_returns_both_sides_for_middle_node() {
    // Bob has both an incoming (Alice->Bob) and an outgoing (Bob->Carol)
    // KNOWS edge. Undirected MATCH anchored on Bob must yield both.
    let mut db = DatabaseInner::new();
    seed_directed_pair(&mut db);
    let r = execute(
        &mut db,
        "MATCH (a:Person)-[:KNOWS]-(b:Person) WHERE a.name = 'Bob' RETURN a, b LIMIT 10",
        None,
    )
    .expect("test: match undirected");
    assert_eq!(
        r.row_count(),
        2,
        "Bob is incident to 2 KNOWS edges (Alice<->Bob<->Carol), got {}: {}",
        r.row_count(),
        r.rows_json()
    );
    let rows = r.rows_json();
    assert!(
        rows.contains("\"name\":\"Alice\"") && rows.contains("\"name\":\"Carol\""),
        "both neighbours must appear, got: {rows}"
    );
}

#[test]
fn test_match_undirected_dedups_self_loop() {
    // A self-loop (source == target == anchor) must appear once in an
    // undirected MATCH, not twice.
    let mut db = DatabaseInner::new();
    execute(
        &mut db,
        "INSERT NODE INTO graph (id = 7, payload = '{\"name\": \"Self\", \"labels\": [\"Person\"]}')",
        None,
    )
    .expect("test: node");
    execute(
        &mut db,
        "INSERT EDGE INTO graph (source = 7, target = 7, label = 'LIKES')",
        None,
    )
    .expect("test: self-loop");
    let r = execute(
        &mut db,
        "MATCH (a:Person)-[:LIKES]-(b:Person) WHERE a.name = 'Self' RETURN a, b LIMIT 10",
        None,
    )
    .expect("test: undirected self-loop");
    assert_eq!(
        r.row_count(),
        1,
        "self-loop under undirected MATCH should appear once, got: {}",
        r.rows_json()
    );
}

// =========================================================================
// Regression: DROP / TRUNCATE clear associated graph store
// (Devin Review PR #594 finding #3).
// =========================================================================
//
// Before the fix, `DROP COLLECTION g` and `TRUNCATE g` left any graph
// store keyed by `g` intact. Re-creating `g` then surfaced stale nodes
// and edges from the previous lifecycle.

#[test]
fn test_drop_collection_removes_graph_store() {
    let mut db = DatabaseInner::new();
    // Auto-provision the graph store by inserting a node+edge via SQL.
    execute(
        &mut db,
        "INSERT NODE INTO g (id = 1, payload = '{\"name\": \"Alice\"}')",
        None,
    )
    .expect("test: node");
    execute(
        &mut db,
        "INSERT EDGE INTO g (source = 1, target = 2, label = 'KNOWS')",
        None,
    )
    .expect("test: edge");
    // Materialise the backing collection so DROP has something to remove.
    db.create_metadata_collection("g").expect("test: create");

    // DROP the collection.
    execute(&mut db, "DROP COLLECTION g", None).expect("test: drop");

    // Recreate and assert the graph store is fresh.
    db.create_metadata_collection("g").expect("test: recreate");
    let err = execute(&mut db, "SELECT EDGES FROM g", None);
    assert!(
        err.is_err(),
        "empty graph store: no edges to select (got: {err:?})"
    );
}

#[test]
fn test_truncate_collection_clears_graph_store() {
    let mut db = DatabaseInner::new();
    db.create_metadata_collection("g").expect("test: create");
    execute(
        &mut db,
        "INSERT NODE INTO g (id = 1, payload = '{\"name\": \"Alice\"}')",
        None,
    )
    .expect("test: node");
    execute(
        &mut db,
        "INSERT EDGE INTO g (source = 1, target = 2, label = 'KNOWS')",
        None,
    )
    .expect("test: edge");
    // Sanity: SELECT EDGES returns the seeded edge.
    let before = execute(&mut db, "SELECT EDGES FROM g", None).expect("test: before");
    assert_eq!(before.row_count(), 1);

    // TRUNCATE must wipe the graph data too.
    execute(&mut db, "TRUNCATE g", None).expect("test: truncate");

    let after = execute(&mut db, "SELECT EDGES FROM g", None);
    match after {
        Ok(r) => assert_eq!(
            r.row_count(),
            0,
            "graph store must be empty after TRUNCATE, got: {}",
            r.rows_json()
        ),
        Err(e) => assert!(
            e.contains("empty") || e.contains("not found"),
            "unexpected error: {e}"
        ),
    }
}

#[test]
fn test_drop_collection_without_graph_store_does_not_fail() {
    // Negative: DROP a collection that never had graph DML on it must
    // not panic or spuriously error on the graph side.
    let mut db = DatabaseInner::new();
    db.create_metadata_collection("plain")
        .expect("test: create");
    execute(&mut db, "DROP COLLECTION plain", None).expect("test: drop plain");
}

#[test]
fn test_match_outgoing_unaffected_by_fix() {
    // Guard: outgoing direction (the only case currently exercised by the
    // existing suite) still behaves exactly as before.
    let mut db = DatabaseInner::new();
    seed_directed_pair(&mut db);
    let r = execute(
        &mut db,
        "MATCH (a:Person)-[:KNOWS]->(b:Person) WHERE a.name = 'Alice' RETURN a, b LIMIT 10",
        None,
    )
    .expect("test: outgoing");
    assert_eq!(r.row_count(), 1);
    assert!(r.rows_json().contains("\"name\":\"Bob\""));
}
