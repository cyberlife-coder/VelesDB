use super::*;

#[test]
fn test_upsert_node_sets_labels() {
    let mut g = WasmGraphStore::new();
    g.upsert_node(
        1,
        Some(serde_json::json!({"name": "Alice"})),
        vec!["Person".to_string()],
    );
    let node = g.get_node(1).expect("test: node");
    assert_eq!(node.labels, vec!["Person".to_string()]);
}

#[test]
fn test_insert_edge_auto_id_matches_core_canonical_hash() {
    let mut g = WasmGraphStore::new();
    let a = g
        .insert_edge(None, 1, 2, "KNOWS".to_string(), None)
        .expect("test: insert a");
    let b = g
        .insert_edge(None, 2, 3, "KNOWS".to_string(), None)
        .expect("test: insert b");
    // Auto-assigned ids come from core's canonical derivation, so the
    // same (source, target, label) triple yields the same id everywhere.
    assert_eq!(a, velesdb_core::hash_edge_id(1, 2, "KNOWS"));
    assert_eq!(b, velesdb_core::hash_edge_id(2, 3, "KNOWS"));
    assert_ne!(a, b, "distinct triples must yield distinct ids");
}

#[test]
fn test_delete_edge_returns_true_on_match() {
    let mut g = WasmGraphStore::new();
    let id = g
        .insert_edge(None, 1, 2, "KNOWS".to_string(), None)
        .expect("test: insert");
    assert!(g.delete_edge_by_id(id));
    assert!(!g.delete_edge_by_id(id));
}

#[test]
fn test_filter_edges_by_label() {
    let mut g = WasmGraphStore::new();
    g.insert_edge(None, 1, 2, "KNOWS".to_string(), None)
        .expect("test: knows");
    g.insert_edge(None, 2, 3, "LIKES".to_string(), None)
        .expect("test: likes");
    let hits: Vec<_> = g.filter_edges(None, None, Some("KNOWS")).collect();
    assert_eq!(hits.len(), 1);
}

#[test]
fn test_delete_edges_where() {
    let mut g = WasmGraphStore::new();
    g.insert_edge(None, 1, 2, "KNOWS".to_string(), None)
        .expect("test: e1");
    g.insert_edge(None, 1, 3, "KNOWS".to_string(), None)
        .expect("test: e2");
    g.insert_edge(None, 2, 3, "LIKES".to_string(), None)
        .expect("test: e3");
    let n = g.delete_edges_where(|e| e.source == 1);
    assert_eq!(n, 2);
    assert_eq!(g.edges().len(), 1);
}

#[test]
fn test_nodes_with_label() {
    let mut g = WasmGraphStore::new();
    g.upsert_node(1, None, vec!["Person".to_string()]);
    g.upsert_node(2, None, vec!["Animal".to_string()]);
    g.upsert_node(3, None, vec!["Person".to_string()]);
    let people = g.nodes_with_label("Person");
    assert_eq!(people.len(), 2);
}

#[test]
fn test_auto_id_is_independent_of_prior_explicit_id() {
    let mut g = WasmGraphStore::new();
    g.insert_edge(Some(100), 1, 2, "X".to_string(), None)
        .expect("test: explicit id");
    // A following auto insert derives its id from its own triple, not
    // from any monotonic counter influenced by the explicit id.
    let next = g
        .insert_edge(None, 2, 3, "Y".to_string(), None)
        .expect("test: next");
    assert_eq!(next, velesdb_core::hash_edge_id(2, 3, "Y"));
}

// --- Finding J: duplicate explicit edge id rejection -----------------

#[test]
fn test_insert_edge_with_duplicate_explicit_id_returns_error() {
    let mut g = WasmGraphStore::new();
    g.insert_edge(Some(1), 1, 2, "KNOWS".to_string(), None)
        .expect("test: first insert");
    let err = g.insert_edge(Some(1), 3, 4, "KNOWS".to_string(), None);
    assert!(err.is_err(), "duplicate explicit id must be rejected");
    let msg = err.expect_err("test: err");
    assert!(
        msg.contains("already exists") && msg.contains('1'),
        "error should mention existing id, got: {msg}"
    );
    // Store unchanged: only the first edge should exist.
    assert_eq!(g.edges().len(), 1);
}

#[test]
fn test_insert_edge_with_auto_assigned_id_never_collides() {
    // Auto-assigned ids derive from the canonical (source, target, label)
    // hash; distinct triples yield distinct ids, so mixing one explicit
    // edge with several distinct auto edges stays collision-free.
    let mut g = WasmGraphStore::new();
    g.insert_edge(Some(42), 1, 2, "KNOWS".to_string(), None)
        .expect("test: explicit");
    for src in 10..20u64 {
        g.insert_edge(None, src, src + 1, "R".to_string(), None)
            .expect("test: auto");
    }
    // 1 explicit + 10 auto = 11 edges, all distinct ids.
    assert_eq!(g.edges().len(), 11);
    let mut ids: Vec<u64> = g.edges().iter().map(|e| e.id).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 11, "every id must be unique");
}

#[test]
fn test_insert_edge_after_delete_can_reuse_same_explicit_id() {
    let mut g = WasmGraphStore::new();
    g.insert_edge(Some(7), 1, 2, "KNOWS".to_string(), None)
        .expect("test: first");
    assert!(g.delete_edge_by_id(7));
    // Once freed, the explicit id is reusable.
    g.insert_edge(Some(7), 5, 6, "KNOWS".to_string(), None)
        .expect("test: reuse after delete");
    assert_eq!(g.edges().len(), 1);
    assert_eq!(g.edges()[0].source, 5);
}
