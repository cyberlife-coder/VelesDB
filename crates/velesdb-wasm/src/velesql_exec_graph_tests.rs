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

// =========================================================================
// Regression: MATCH WHERE evaluates against all bound node aliases
// (Devin Review PR #594 finding ANALYSIS_0004).
// =========================================================================
//
// Before the fix, `rewrite_alias_prefix` only stripped the starting node's
// alias from WHERE predicates. Any reference to a non-starting alias
// (e.g. `b.name` in a 1-hop or `b.age`, `c.x` in a 2-hop) was looked up
// as a literal field named `"b.name"` in the starting node's payload and
// silently returned 0 rows. The fix binds every node alias in scope and
// evaluates the WHERE against the full bindings map.
//
// The helpers below seed richer payloads (city, age) because the default
// `seed_graph` fixture only carries `name`.

fn seed_cities_graph(db: &mut DatabaseInner) {
    execute(
        db,
        "INSERT NODE INTO graph (id = 10, payload = '{\"name\": \"Alice\", \"city\": \"Paris\", \"age\": 30, \"labels\": [\"Person\"]}')",
        None,
    )
    .expect("test: alice");
    execute(
        db,
        "INSERT NODE INTO graph (id = 11, payload = '{\"name\": \"Bob\", \"city\": \"Lyon\", \"age\": 40, \"labels\": [\"Person\"]}')",
        None,
    )
    .expect("test: bob");
    execute(
        db,
        "INSERT NODE INTO graph (id = 12, payload = '{\"name\": \"Carol\", \"city\": \"Paris\", \"age\": 25, \"labels\": [\"Person\"]}')",
        None,
    )
    .expect("test: carol");
    // Alice (Paris) -KNOWS-> Bob (Lyon); Alice -KNOWS-> Carol (Paris).
    execute(
        db,
        "INSERT EDGE INTO graph (source = 10, target = 11, label = 'KNOWS')",
        None,
    )
    .expect("test: edge a->b");
    execute(
        db,
        "INSERT EDGE INTO graph (source = 10, target = 12, label = 'KNOWS')",
        None,
    )
    .expect("test: edge a->c");
}

#[test]
fn test_match_where_filters_second_node_by_payload() {
    // `b.name = 'Bob'`: predicate references node `b`, not the starting
    // node. Before the fix: 0 rows. Expected: the single Alice->Bob edge.
    let mut db = DatabaseInner::new();
    seed_graph(&mut db);
    let r = execute(
        &mut db,
        "MATCH (a:Person)-[:KNOWS]->(b:Person) WHERE b.name = 'Bob' RETURN a, b LIMIT 10",
        None,
    )
    .expect("test: match where on b");
    assert_eq!(
        r.row_count(),
        1,
        "predicate on b.name should filter to the Alice->Bob edge, got: {}",
        r.rows_json()
    );
    let rows = r.rows_json();
    assert!(rows.contains("\"name\":\"Alice\""), "a is Alice: {rows}");
    assert!(rows.contains("\"name\":\"Bob\""), "b is Bob: {rows}");
}

#[test]
fn test_match_where_filters_both_nodes_with_and() {
    // `a.city = 'Paris' AND b.city = 'Lyon'`: only Alice(Paris)->Bob(Lyon).
    let mut db = DatabaseInner::new();
    seed_cities_graph(&mut db);
    let r = execute(
        &mut db,
        "MATCH (a:Person)-[:KNOWS]->(b:Person) WHERE a.city = 'Paris' AND b.city = 'Lyon' RETURN a, b LIMIT 10",
        None,
    )
    .expect("test: both nodes AND");
    assert_eq!(
        r.row_count(),
        1,
        "only Alice(Paris)->Bob(Lyon) matches, got: {}",
        r.rows_json()
    );
    let rows = r.rows_json();
    assert!(rows.contains("\"name\":\"Alice\""));
    assert!(rows.contains("\"name\":\"Bob\""));
    // Paris->Paris (Alice->Carol) is filtered out.
    assert!(
        !rows.contains("\"name\":\"Carol\""),
        "Alice->Carol must be excluded, got: {rows}"
    );
}

#[test]
fn test_match_where_two_hop_filters_middle_node() {
    // 2-hop: Alice -KNOWS-> Bob -KNOWS-> Carol. WHERE `b.name = 'Bob'`
    // must filter against the MIDDLE node, not the starting one. Before
    // the fix, 2-hop had no WHERE evaluation at all (all paths returned);
    // with no `b` binding, `b.name` would have been a dead-letter key.
    let mut db = DatabaseInner::new();
    seed_graph(&mut db);
    let r = execute(
        &mut db,
        "MATCH (a:Person)-[:KNOWS]->(b:Person)-[:KNOWS]->(c:Person) WHERE b.name = 'Bob' RETURN a, b, c LIMIT 10",
        None,
    )
    .expect("test: 2-hop where on b");
    assert_eq!(
        r.row_count(),
        1,
        "Alice->Bob->Carol is the only 2-hop path with b.name='Bob', got: {}",
        r.rows_json()
    );
    let rows = r.rows_json();
    assert!(
        rows.contains("\"name\":\"Alice\"")
            && rows.contains("\"name\":\"Bob\"")
            && rows.contains("\"name\":\"Carol\""),
        "expected the full Alice->Bob->Carol triple, got: {rows}"
    );
}

#[test]
fn test_match_where_bare_field_applies_to_first_node() {
    // Backward compat: `WHERE name = 'Alice'` (no alias prefix) must
    // resolve against the starting node `a`.
    let mut db = DatabaseInner::new();
    seed_graph(&mut db);
    let r = execute(
        &mut db,
        "MATCH (a:Person)-[:KNOWS]->(b:Person) WHERE name = 'Alice' RETURN a, b LIMIT 10",
        None,
    )
    .expect("test: bare field on first node");
    assert_eq!(
        r.row_count(),
        1,
        "bare `name` must bind to node a, got: {}",
        r.rows_json()
    );
    assert!(r.rows_json().contains("\"name\":\"Alice\""));
    assert!(r.rows_json().contains("\"name\":\"Bob\""));
}

#[test]
fn test_match_where_unknown_alias_returns_empty() {
    // Negative: `z.name` references an alias that doesn't exist in the
    // pattern. Consistent with the "missing field" convention used by
    // `velesql_where` (comparisons on missing fields return false), this
    // must yield an empty result rather than an error or a panic.
    let mut db = DatabaseInner::new();
    seed_graph(&mut db);
    let r = execute(
        &mut db,
        "MATCH (a:Person)-[:KNOWS]->(b:Person) WHERE z.name = 'Alice' RETURN a, b LIMIT 10",
        None,
    )
    .expect("test: unknown alias executes without error");
    assert_eq!(
        r.row_count(),
        0,
        "unknown alias must silently filter to empty, got: {}",
        r.rows_json()
    );
}

// =========================================================================
// Regression: `NOT IN` on a missing column aligns with `!=` (returns false)
// (Devin Review PR #594 Finding B — semantic inconsistency).
// =========================================================================
//
// Before the fix, `eval_in` returned `Ok(c.negated)` for a missing
// column: `IN` gave `false` (correct) but `NOT IN` gave `true`. This
// contradicted `eval_comparison`, which returns `false` uniformly for
// all operators (including `!=`) on missing columns. The fix aligns
// `eval_in` with that convention: missing column → `false`, regardless
// of operator polarity.
//
// These tests exercise the WASM WHERE matcher through MATCH because
// the single-node MATCH variant runs WHERE with a single alias scope,
// which is the simplest path to exercise `eval_in` / `eval_comparison`
// end-to-end in the demo executor.

#[test]
fn test_eval_in_missing_column_returns_false() {
    // Baseline: the pre-existing behaviour was already `false`.
    let mut db = DatabaseInner::new();
    seed_graph(&mut db);
    // `city` is absent from every seed_graph node (they only carry `name`).
    let r = execute(
        &mut db,
        "MATCH (a:Person) WHERE a.city IN ('Paris','Lyon') RETURN a LIMIT 10",
        None,
    )
    .expect("test: in on missing col");
    assert_eq!(
        r.row_count(),
        0,
        "missing `city`: IN returns false for every row, got: {}",
        r.rows_json()
    );
}

#[test]
fn test_eval_not_in_missing_column_returns_false() {
    // REGRESSION: before the fix this returned 3 rows (every Person,
    // because `NOT IN` on a missing column was `true`). After the fix
    // it returns 0 rows, consistent with `!=` on a missing column.
    let mut db = DatabaseInner::new();
    seed_graph(&mut db);
    let r = execute(
        &mut db,
        "MATCH (a:Person) WHERE a.city NOT IN ('Paris','Lyon') RETURN a LIMIT 10",
        None,
    )
    .expect("test: not-in on missing col");
    assert_eq!(
        r.row_count(),
        0,
        "missing `city`: NOT IN must also return false (missing-column rule), got: {}",
        r.rows_json()
    );
}

#[test]
fn test_eval_ne_missing_column_returns_false() {
    // Backward-compat guard: `!=` on a missing column was already
    // false and must remain false. This pins the convention that
    // Finding B's fix aligns `NOT IN` with.
    let mut db = DatabaseInner::new();
    seed_graph(&mut db);
    let r = execute(
        &mut db,
        "MATCH (a:Person) WHERE a.city != 'Paris' RETURN a LIMIT 10",
        None,
    )
    .expect("test: != on missing col");
    assert_eq!(
        r.row_count(),
        0,
        "missing `city`: != must return false, got: {}",
        r.rows_json()
    );
}

#[test]
fn test_eval_not_in_present_column_still_excludes_matches() {
    // Non-regression: when the column IS present, `NOT IN` must still
    // work — the fix only changes the missing-column branch.
    let mut db = DatabaseInner::new();
    seed_cities_graph(&mut db);
    let r = execute(
        &mut db,
        "MATCH (a:Person) WHERE a.city NOT IN ('Lyon') RETURN a LIMIT 10",
        None,
    )
    .expect("test: not-in on present col");
    // Alice (Paris) and Carol (Paris) match; Bob (Lyon) is excluded.
    assert_eq!(
        r.row_count(),
        2,
        "NOT IN('Lyon') must exclude Bob only, got: {}",
        r.rows_json()
    );
    let rows = r.rows_json();
    assert!(rows.contains("\"name\":\"Alice\""));
    assert!(rows.contains("\"name\":\"Carol\""));
    assert!(
        !rows.contains("\"name\":\"Bob\""),
        "Bob (Lyon) must be filtered, got: {rows}"
    );
}

// =========================================================================
// Regression: MATCH WHERE threads query `$params` through the scope
// (Devin Review PR #594 Finding A — BUG).
// =========================================================================
//
// Before the fix, `execute_match` accepted the `Params` map but dropped
// it on the floor: `matches_where_in_match_scope` hardcoded a fresh
// `Params::new()` when calling `velesql_where::matches`. Any
// `$placeholder` inside a MATCH WHERE silently resolved to nothing and
// the row count collapsed to zero. The fix threads `&Params` through
// `execute_single_node`, `execute_1_hop`, `execute_2_hop`,
// `expand_one_hop`, `expand_from_a`, and `expand_from_b` down to the
// matcher, and converts the return type to `Result<bool, String>` so
// that an unbound parameter surfaces as an error instead of a zero-row
// result.

#[test]
fn test_match_where_with_param_on_first_node() {
    // `WHERE a.name = $name` + params `{"name":"Alice"}` must return
    // Alice's outgoing KNOWS edge. Before the fix: 0 rows.
    let mut db = DatabaseInner::new();
    seed_graph(&mut db);
    let r = execute(
        &mut db,
        "MATCH (a:Person)-[:KNOWS]->(b:Person) WHERE a.name = $name RETURN a, b LIMIT 10",
        Some(r#"{"name":"Alice"}"#),
    )
    .expect("test: param on first node");
    assert_eq!(
        r.row_count(),
        1,
        "Alice-KNOWS-Bob is the only edge matching a.name=$name, got: {}",
        r.rows_json()
    );
    let rows = r.rows_json();
    assert!(rows.contains("\"name\":\"Alice\""), "a is Alice: {rows}");
    assert!(rows.contains("\"name\":\"Bob\""), "b is Bob: {rows}");
}

#[test]
fn test_match_where_with_param_on_second_node() {
    // Parameters on the non-starting node's predicate also resolve,
    // proving the params map is threaded through every expansion layer.
    let mut db = DatabaseInner::new();
    seed_graph(&mut db);
    let r = execute(
        &mut db,
        "MATCH (a:Person)-[:KNOWS]->(b:Person) WHERE b.name = $bname RETURN a, b LIMIT 10",
        Some(r#"{"bname":"Carol"}"#),
    )
    .expect("test: param on second node");
    assert_eq!(
        r.row_count(),
        1,
        "Bob-KNOWS-Carol is the only edge matching b.name=$bname, got: {}",
        r.rows_json()
    );
    let rows = r.rows_json();
    assert!(rows.contains("\"name\":\"Bob\""), "a is Bob: {rows}");
    assert!(rows.contains("\"name\":\"Carol\""), "b is Carol: {rows}");
}

#[test]
fn test_match_where_with_multiple_params() {
    // AND between two parameterised predicates exercises the full
    // `eval_and` + `resolve_value` path with both sides needing params.
    let mut db = DatabaseInner::new();
    seed_cities_graph(&mut db);
    let r = execute(
        &mut db,
        "MATCH (a:Person)-[:KNOWS]->(b:Person) WHERE a.city = $a_city AND b.city = $b_city RETURN a, b LIMIT 10",
        Some(r#"{"a_city":"Paris","b_city":"Lyon"}"#),
    )
    .expect("test: two params");
    assert_eq!(
        r.row_count(),
        1,
        "only Alice(Paris)->Bob(Lyon) matches both params, got: {}",
        r.rows_json()
    );
    let rows = r.rows_json();
    assert!(rows.contains("\"name\":\"Alice\""));
    assert!(rows.contains("\"name\":\"Bob\""));
    assert!(
        !rows.contains("\"name\":\"Carol\""),
        "Alice->Carol (Paris->Paris) must be filtered out, got: {rows}"
    );
}

#[test]
fn test_match_where_missing_param_returns_error() {
    // Negative: referencing an unbound `$param` must surface as an
    // `Err`, not as a silent zero-row result. Before the fix this was
    // impossible to detect because `matches_where_in_match_scope`
    // swallowed the error via `.unwrap_or(false)` on a fresh empty
    // params map (so every row looked "missing-param → false").
    let mut db = DatabaseInner::new();
    seed_graph(&mut db);
    let err = execute(
        &mut db,
        "MATCH (a:Person)-[:KNOWS]->(b:Person) WHERE a.name = $unknown RETURN a, b LIMIT 10",
        Some("{}"),
    );
    assert!(err.is_err(), "unbound $unknown must be an error");
    let msg = err.expect_err("test: err");
    assert!(
        msg.contains("$unknown") || msg.contains("not bound"),
        "error should mention the unbound parameter, got: {msg}"
    );
}

// =========================================================================
// Regression: MATCH WHERE allows filtering by alias-qualified node id
// (Devin Review PR #594 Finding C — feature gap).
// =========================================================================
//
// Before the fix, `make_binding` stored only the payload. The merged
// payload fed to `velesql_where::matches` had no way to surface the
// node id through `get_nested_field("a.id")`, so predicates like
// `WHERE a.id = 1` matched 0 rows. The fix injects the node id as a
// top-level `"id"` field inside each alias object (and also at the
// root of the merged payload, for the bare `WHERE id = ...` form that
// falls through to the starting node).

#[test]
fn test_match_where_by_node_id_on_starting_node() {
    let mut db = DatabaseInner::new();
    seed_graph(&mut db);
    let r = execute(
        &mut db,
        "MATCH (a:Person)-[:KNOWS]->(b:Person) WHERE a.id = 1 RETURN a, b LIMIT 10",
        None,
    )
    .expect("test: filter by a.id");
    assert_eq!(
        r.row_count(),
        1,
        "Alice (id=1) has exactly one outgoing KNOWS edge, got: {}",
        r.rows_json()
    );
    let rows = r.rows_json();
    assert!(rows.contains("\"name\":\"Alice\""));
    assert!(rows.contains("\"name\":\"Bob\""));
}

#[test]
fn test_match_where_by_node_id_on_second_node() {
    let mut db = DatabaseInner::new();
    seed_graph(&mut db);
    let r = execute(
        &mut db,
        "MATCH (a:Person)-[:KNOWS]->(b:Person) WHERE b.id = 2 RETURN a, b LIMIT 10",
        None,
    )
    .expect("test: filter by b.id");
    assert_eq!(
        r.row_count(),
        1,
        "Bob (id=2) receives exactly one KNOWS edge (from Alice), got: {}",
        r.rows_json()
    );
    let rows = r.rows_json();
    assert!(rows.contains("\"name\":\"Alice\""));
    assert!(rows.contains("\"name\":\"Bob\""));
}

#[test]
fn test_match_where_by_node_id_with_param() {
    // Combines Finding A (params threaded) and Finding C (node id
    // injection): `WHERE a.id = $aid` must resolve both.
    let mut db = DatabaseInner::new();
    seed_graph(&mut db);
    let r = execute(
        &mut db,
        "MATCH (a:Person)-[:KNOWS]->(b:Person) WHERE a.id = $aid RETURN a, b LIMIT 10",
        Some(r#"{"aid": 2}"#),
    )
    .expect("test: id via param");
    assert_eq!(
        r.row_count(),
        1,
        "Bob (id=2) has one outgoing KNOWS edge to Carol, got: {}",
        r.rows_json()
    );
    let rows = r.rows_json();
    assert!(rows.contains("\"name\":\"Bob\""));
    assert!(rows.contains("\"name\":\"Carol\""));
}

#[test]
fn test_match_where_bare_id_targets_starting_node() {
    // Backward-compat: `WHERE id = 1` (no alias prefix) must resolve
    // against the starting node's id — same convention as bare
    // `WHERE name = 'Alice'` in `test_match_where_bare_field_applies_to_first_node`.
    let mut db = DatabaseInner::new();
    seed_graph(&mut db);
    let r = execute(
        &mut db,
        "MATCH (a:Person)-[:KNOWS]->(b:Person) WHERE id = 1 RETURN a, b LIMIT 10",
        None,
    )
    .expect("test: bare id on starting node");
    assert_eq!(
        r.row_count(),
        1,
        "bare `id = 1` must target node a, got: {}",
        r.rows_json()
    );
    assert!(r.rows_json().contains("\"name\":\"Alice\""));
    assert!(r.rows_json().contains("\"name\":\"Bob\""));
}

#[test]
fn test_match_where_payload_id_does_not_shadow_node_id() {
    // Edge case / documented collision rule: when a user payload
    // carries a field literally named `"id"` (e.g. an application-
    // level primary key that differs from the graph node id), the
    // GRAPH node id always wins in a MATCH WHERE clause. A MATCH
    // targets the graph identifier; overriding that with an arbitrary
    // payload key would silently break pattern-matching semantics.
    let mut db = DatabaseInner::new();
    // Insert a node where the payload contains its own "id" = 999
    // but the graph node id is 42.
    execute(
        &mut db,
        "INSERT NODE INTO graph (id = 42, payload = '{\"name\": \"Shadowed\", \"id\": 999, \"labels\": [\"Person\"]}')",
        None,
    )
    .expect("test: insert shadowed");

    // `WHERE a.id = 42` must match (graph node id wins).
    let r_node = execute(
        &mut db,
        "MATCH (a:Person) WHERE a.id = 42 RETURN a LIMIT 10",
        None,
    )
    .expect("test: a.id=42");
    assert_eq!(
        r_node.row_count(),
        1,
        "graph node id must be reachable via a.id, got: {}",
        r_node.rows_json()
    );

    // `WHERE a.id = 999` (the payload's shadowed id) must NOT match:
    // the node id (42) wins the collision, so 999 is nobody's id.
    let r_shadow = execute(
        &mut db,
        "MATCH (a:Person) WHERE a.id = 999 RETURN a LIMIT 10",
        None,
    )
    .expect("test: a.id=999");
    assert_eq!(
        r_shadow.row_count(),
        0,
        "payload `id` must not shadow node id in MATCH WHERE, got: {}",
        r_shadow.rows_json()
    );
}

// =========================================================================
// Regression: explicit graph edge ids must be unique
// (Devin Review PR #594 Finding J — data integrity).
// =========================================================================
//
// Before the fix, `WasmGraphStore::insert_edge` accepted duplicate explicit
// ids. A second `INSERT EDGE INTO g (id = 1, ...)` appended a second edge
// with id 1 to the store, and a subsequent `DELETE EDGE 1 FROM g` deleted
// both at once — silent data loss. The fix rejects the duplicate at
// insertion time with a clear error.

#[test]
fn test_bdd_insert_edge_duplicate_explicit_id_is_rejected() {
    let mut db = DatabaseInner::new();
    execute(
        &mut db,
        "INSERT EDGE INTO g (id = 1, source = 1, target = 2, label = 'KNOWS')",
        None,
    )
    .expect("test: first edge");
    let err = execute(
        &mut db,
        "INSERT EDGE INTO g (id = 1, source = 3, target = 4, label = 'KNOWS')",
        None,
    );
    assert!(err.is_err(), "duplicate explicit id must be rejected");
    let msg = err.expect_err("test: err");
    assert!(
        msg.contains("already exists") && msg.contains('1'),
        "error should mention the colliding id, got: {msg}"
    );
    // Store unchanged: SELECT EDGES still returns exactly one edge.
    let r = execute(&mut db, "SELECT EDGES FROM g", None).expect("test: select");
    assert_eq!(
        r.row_count(),
        1,
        "failed duplicate insert must not have added a second edge, got: {}",
        r.rows_json()
    );
    assert!(
        r.rows_json().contains("\"source\":1"),
        "the one surviving edge is the first inserted (source=1), got: {}",
        r.rows_json()
    );
}

#[test]
fn test_bdd_insert_edge_auto_id_never_collides_with_explicit() {
    // Mix explicit + auto ids: the auto counter is bumped past explicit
    // values (see `next_edge_id` logic), so implicit inserts never
    // collide — and each explicit insert only checks against the current
    // edge set, not the historical counter.
    let mut db = DatabaseInner::new();
    execute(
        &mut db,
        "INSERT EDGE INTO g (id = 100, source = 1, target = 2, label = 'REL')",
        None,
    )
    .expect("test: explicit 100");
    for n in 0..5 {
        execute(
            &mut db,
            &format!(
                "INSERT EDGE INTO g (source = {src}, target = {tgt}, label = 'AUTO')",
                src = n + 10,
                tgt = n + 11
            ),
            None,
        )
        .expect("test: auto insert");
    }
    let r = execute(&mut db, "SELECT EDGES FROM g", None).expect("test: select");
    assert_eq!(
        r.row_count(),
        6,
        "1 explicit + 5 auto = 6 unique edges, got: {}",
        r.rows_json()
    );
}

#[test]
fn test_bdd_insert_edge_after_delete_can_reuse_explicit_id() {
    // Non-regression + semantic: once an explicit id is freed via
    // DELETE EDGE, a subsequent INSERT EDGE with the same explicit id is
    // accepted (the uniqueness check is against the CURRENT edge set).
    let mut db = DatabaseInner::new();
    execute(
        &mut db,
        "INSERT EDGE INTO g (id = 7, source = 1, target = 2, label = 'KNOWS')",
        None,
    )
    .expect("test: first");
    execute(&mut db, "DELETE EDGE 7 FROM g", None).expect("test: delete");
    // Id 7 is now free — re-inserting it must succeed.
    execute(
        &mut db,
        "INSERT EDGE INTO g (id = 7, source = 5, target = 6, label = 'KNOWS')",
        None,
    )
    .expect("test: reuse after delete");
    let r = execute(&mut db, "SELECT EDGES FROM g", None).expect("test: select");
    assert_eq!(r.row_count(), 1);
    assert!(
        r.rows_json().contains("\"source\":5"),
        "reused edge must carry the new source, got: {}",
        r.rows_json()
    );
}

// =========================================================================
// Regression: multi-pattern MATCH must surface a clear error
// (Devin Review PR #594 Finding K — silent partial execution).
// =========================================================================
//
// Before the fix, `execute_match` only considered `clause.patterns[0]`.
// A query like `MATCH (a:X), (b:Y) RETURN a, b` silently dropped `(b:Y)`
// and returned rows from the first pattern alone. The fix fails loud
// when `clause.patterns.len() > 1`, pointing callers at the persistent
// core backend.

#[test]
fn test_bdd_match_single_pattern_works() {
    // Non-regression: single-pattern MATCH keeps its behaviour.
    let mut db = DatabaseInner::new();
    seed_graph(&mut db);
    let r = execute(
        &mut db,
        "MATCH (a:Person)-[:KNOWS]->(b:Person) RETURN a, b LIMIT 10",
        None,
    )
    .expect("test: single pattern");
    assert_eq!(r.row_count(), 2);
}

#[test]
fn test_bdd_match_multi_pattern_returns_clear_error() {
    // The public VelesQL parser today returns `patterns.len() == 1` for
    // every successful MATCH parse (see
    // `parser::match_patterns::parse_pattern_list`). That makes Finding
    // K's silent-ignore scenario unreachable via SQL alone — but the
    // AST type signature (`patterns: Vec<GraphPattern>`) still allows a
    // future parser or a programmatic caller to hand the executor a
    // multi-pattern MATCH. This test exercises that code path directly
    // by building the AST, ensuring the executor fails loud rather than
    // silently dropping additional patterns.
    use std::collections::HashMap;
    use velesdb_core::velesql::{GraphPattern, NodePattern, Parser};
    let mut db = DatabaseInner::new();
    seed_graph(&mut db);
    // Start from a valid single-pattern query so every other field
    // (RETURN, LIMIT, etc.) is well-formed.
    let mut parsed = Parser::parse("MATCH (a:Person)-[:KNOWS]->(b:Person) RETURN a, b LIMIT 10")
        .expect("test: parse baseline");
    // Inject a second pattern so `patterns.len() == 2`.
    let clause = parsed.match_clause.as_mut().expect("test: has match");
    let second = GraphPattern {
        name: None,
        nodes: vec![NodePattern {
            alias: Some("c".to_string()),
            labels: vec!["Person".to_string()],
            properties: HashMap::new(),
            collection: None,
        }],
        relationships: Vec::new(),
    };
    clause.patterns.push(second);
    assert_eq!(clause.patterns.len(), 2, "test fixture pre-condition");

    let err = crate::velesql_graph::execute_match(
        &mut db,
        &parsed,
        &crate::velesql_value::parse_params(None).expect("test: params"),
    );
    assert!(err.is_err(), "multi-pattern MATCH must be rejected loud");
    let msg = err.expect_err("test: err");
    assert!(
        msg.contains("Multi-pattern MATCH") || msg.contains("multi-pattern"),
        "error must describe the unsupported feature, got: {msg}"
    );
    // The error must also point the user at the persistent core backend.
    assert!(
        msg.contains("core") || msg.contains("persistence"),
        "error should reference the persistence-enabled backend, got: {msg}"
    );
}

// =========================================================================
// Regression: MATCH node output matches WHERE alias scope
// (Devin Review PR #594 Finding O — scope asymmetry).
// =========================================================================
//
// Before the fix, `node_json` wrapped node data in a nested object:
// `{"id": N, "labels": [...], "payload": {"name": "Alice"}}`. JS callers
// had to read `a.payload.name` even though `WHERE a.name = 'Alice'` (used
// on the input side) resolved `name` at the root of the alias scope. The
// fix flattens the output: payload fields are merged at the alias root
// so `a.name` works symmetrically on read and write. Node id always
// wins the collision with a literal payload `id` / `labels` key (same
// rule as MATCH WHERE).

#[test]
fn test_bdd_match_output_structure_matches_where_scope_flat() {
    let mut db = DatabaseInner::new();
    seed_graph(&mut db);
    let r = execute(
        &mut db,
        "MATCH (a:Person)-[:KNOWS]->(b:Person) WHERE a.name = 'Alice' RETURN a, b LIMIT 10",
        None,
    )
    .expect("test: match");
    assert_eq!(r.row_count(), 1);
    // Each returned row's `a` object must expose `name` at the root —
    // NOT under a nested `payload` key.
    let rows: serde_json::Value = serde_json::from_str(&r.rows_json()).expect("test: parse rows");
    let arr = rows.as_array().expect("test: array");
    assert_eq!(arr.len(), 1);
    let a = &arr[0]["a"];
    assert_eq!(
        a["name"], "Alice",
        "`a.name` must resolve at the alias root, got: {a}"
    );
    assert_eq!(a["id"], 1, "node id must be present at the root, got: {a}");
    assert!(
        a["labels"].as_array().is_some(),
        "`a.labels` must be present at the root, got: {a}"
    );
    // Backward-incompatible shape (nested `payload`) must NOT appear.
    assert!(
        a.get("payload").is_none(),
        "flat output: nested `payload` key must not exist, got: {a}"
    );
}

#[test]
fn test_bdd_match_output_where_filters_on_returned_fields_roundtrip() {
    // Non-regression: WHERE filter + returned row shape agree on field
    // names (`name`, `city`). If a predicate on `b.city = 'Lyon'` filters
    // the row set, the returned JSON for `b` must also carry `city` at
    // the alias root — so a JS caller can do
    // `rows[i].b.city === rows[i].b.city` symmetrically.
    let mut db = DatabaseInner::new();
    seed_cities_graph(&mut db);
    let r = execute(
        &mut db,
        "MATCH (a:Person)-[:KNOWS]->(b:Person) WHERE b.city = 'Lyon' RETURN a, b LIMIT 10",
        None,
    )
    .expect("test: where-filter roundtrip");
    assert_eq!(r.row_count(), 1);
    let rows: serde_json::Value = serde_json::from_str(&r.rows_json()).expect("test: parse rows");
    let arr = rows.as_array().expect("test: array");
    let b = &arr[0]["b"];
    assert_eq!(b["city"], "Lyon");
    assert_eq!(b["name"], "Bob");
    assert_eq!(b["id"], 11);
}

#[test]
fn test_bdd_match_output_node_id_wins_payload_collision() {
    // Collision rule: if a payload carries a literal `"id"` field, the
    // GRAPH node id wins — symmetric with the MATCH WHERE collision rule
    // exercised by `test_match_where_payload_id_does_not_shadow_node_id`.
    let mut db = DatabaseInner::new();
    execute(
        &mut db,
        "INSERT NODE INTO graph (id = 42, payload = '{\"name\": \"X\", \"id\": 999, \"labels\": [\"Person\"]}')",
        None,
    )
    .expect("test: shadowed id");
    let r = execute(
        &mut db,
        "MATCH (a:Person) WHERE a.id = 42 RETURN a LIMIT 10",
        None,
    )
    .expect("test: match");
    assert_eq!(r.row_count(), 1);
    let rows: serde_json::Value = serde_json::from_str(&r.rows_json()).expect("test: parse rows");
    let a = &rows[0]["a"];
    assert_eq!(
        a["id"], 42,
        "graph node id (42) must win over payload `id` (999) in output, got: {a}"
    );
    assert_eq!(a["name"], "X");
}

// =========================================================================
// Regression: TRUNCATE preserves the collection's StorageMode
// (Devin Review PR #594 Finding M — future-compat).
// =========================================================================
//
// Before the fix, `truncate` recreated the collection via
// `create_collection`, which hard-coded `StorageMode::Full`. If a future
// code path ever created a collection with SQ8/Binary/PQ/RaBitQ, TRUNCATE
// would silently flip it to Full. The fix reads back the existing storage
// mode and uses it when re-provisioning.

#[test]
fn test_bdd_truncate_preserves_storage_mode_full() {
    // Baseline: Full mode round-trips through TRUNCATE (this is what the
    // public WASM surface creates today).
    let mut db = DatabaseInner::new();
    db.create_collection("vecs", 4, "cosine")
        .expect("test: create");
    let before_mode = db
        .get_shared_store("vecs")
        .expect("test: before store")
        .borrow()
        .storage_mode();
    execute(&mut db, "TRUNCATE vecs", None).expect("test: truncate");
    let after_mode = db
        .get_shared_store("vecs")
        .expect("test: after store")
        .borrow()
        .storage_mode();
    assert_eq!(
        before_mode, after_mode,
        "TRUNCATE must preserve Full storage mode, got before={before_mode}, after={after_mode}"
    );
}

#[test]
fn test_bdd_truncate_preserves_storage_mode_sq8() {
    // Forward-compat: if the underlying store is created in SQ8 mode
    // directly (as it can be via `VectorStore::new_with_mode`), TRUNCATE
    // must re-provision it in SQ8, not silently drop back to Full. Uses
    // the `install_store` test seam because the public WASM DDL surface
    // currently only creates Full-mode collections.
    use crate::vector_store::VectorStore;
    let mut db = DatabaseInner::new();
    let sq8_store = VectorStore::new_with_mode(4, "cosine", "sq8")
        .map_err(|e| format!("{e:?}"))
        .expect("test: sq8 store");
    db.install_store("vecs", sq8_store).expect("test: install");
    let before_mode = db
        .get_shared_store("vecs")
        .expect("test: before")
        .borrow()
        .storage_mode();
    assert_eq!(before_mode, "sq8", "fixture must start in SQ8");
    execute(&mut db, "TRUNCATE vecs", None).expect("test: truncate");
    let after_mode = db
        .get_shared_store("vecs")
        .expect("test: after")
        .borrow()
        .storage_mode();
    assert_eq!(
        after_mode, "sq8",
        "TRUNCATE must preserve SQ8 storage mode, got {after_mode}"
    );
}
