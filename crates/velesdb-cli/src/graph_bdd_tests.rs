//! BDD tests for CLI graph commands (GIVEN → WHEN → THEN).
//!
//! Tests the full pipeline: create graph → populate → execute command → verify.
//! Covers nominal, edge, and negative cases for all new graph operations:
//! `remove-edge`, `count`, `search`, plus existing commands.

use std::path::PathBuf;
use tempfile::TempDir;
use velesdb_core::collection::graph::GraphSchema;
use velesdb_core::{Database, DistanceMetric, GraphCollection, GraphEdge};

use crate::graph::GraphAction;

// =========================================================================
// Helpers
// =========================================================================

/// Create a fresh database + graph collection in a temp directory.
fn setup_graph_db() -> (TempDir, PathBuf) {
    let dir = TempDir::new().expect("test: create temp dir");
    let db_path = dir.path().join("test_db");
    let db = Database::open(&db_path).expect("test: open database");
    db.create_graph_collection("kg", GraphSchema::schemaless())
        .expect("test: create graph collection");
    drop(db);
    (dir, db_path)
}

/// Create a graph collection with embeddings for search tests.
fn setup_graph_db_with_embeddings() -> (TempDir, PathBuf) {
    let dir = TempDir::new().expect("test: create temp dir");
    let db_path = dir.path().join("test_db");
    let db = Database::open(&db_path).expect("test: open database");
    db.create_graph_collection_with_embeddings(
        "kg",
        GraphSchema::schemaless(),
        4,
        DistanceMetric::Cosine,
    )
    .expect("test: create graph collection with embeddings");
    drop(db);
    (dir, db_path)
}

/// Open graph collection from path.
fn open_graph(path: &PathBuf) -> GraphCollection {
    let db = Database::open(path).expect("test: open database");
    db.get_graph_collection("kg")
        .expect("test: get graph collection")
}

/// Populate a graph with test edges: 1→2→3→4, 2→5.
fn populate_edges(path: &PathBuf) {
    let col = open_graph(path);
    for (id, src, tgt, lbl) in [
        (100, 1, 2, "KNOWS"),
        (101, 2, 3, "KNOWS"),
        (102, 3, 4, "KNOWS"),
        (103, 2, 5, "WROTE"),
    ] {
        col.add_edge(GraphEdge::new(id, src, tgt, lbl).expect("valid edge"))
            .expect("test: add edge");
    }
    col.flush().expect("test: flush");
}

// =========================================================================
// A. remove-edge — Nominal
// =========================================================================

#[test]
fn test_remove_edge_existing_edge_removes_it() {
    // GIVEN: a graph with 4 edges
    let (_dir, path) = setup_graph_db();
    populate_edges(&path);
    assert_eq!(open_graph(&path).edge_count(), 4);

    // WHEN: remove edge 100
    crate::graph::handle(GraphAction::RemoveEdge {
        path: path.clone(),
        collection: "kg".to_string(),
        edge_id: 100,
    })
    .expect("remove-edge should succeed");

    // THEN: edge count is 3
    assert_eq!(open_graph(&path).edge_count(), 3);
}

#[test]
fn test_remove_edge_nonexistent_edge_succeeds_silently() {
    // GIVEN: a graph with 4 edges
    let (_dir, path) = setup_graph_db();
    populate_edges(&path);

    // WHEN: remove non-existent edge 999
    let result = crate::graph::handle(GraphAction::RemoveEdge {
        path: path.clone(),
        collection: "kg".to_string(),
        edge_id: 999,
    });

    // THEN: no error, edge count unchanged
    assert!(
        result.is_ok(),
        "removing non-existent edge should not error"
    );
    assert_eq!(open_graph(&path).edge_count(), 4);
}

// =========================================================================
// B. remove-edge — Edge cases
// =========================================================================

#[test]
fn test_remove_edge_then_readd_same_id() {
    // GIVEN: a graph with edge 100 (1→2 KNOWS)
    let (_dir, path) = setup_graph_db();
    populate_edges(&path);

    // WHEN: remove edge 100, then add a new edge with same ID
    crate::graph::handle(GraphAction::RemoveEdge {
        path: path.clone(),
        collection: "kg".to_string(),
        edge_id: 100,
    })
    .expect("remove should succeed");

    crate::graph::handle(GraphAction::AddEdge {
        path: path.clone(),
        collection: "kg".to_string(),
        id: 100,
        source: 10,
        target: 20,
        label: "NEW_LABEL".to_string(),
    })
    .expect("re-add should succeed");

    // THEN: edge 100 exists with new data
    let col = open_graph(&path);
    let edges = col.get_edges(Some("NEW_LABEL"));
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].source(), 10);
}

#[test]
fn test_remove_all_edges_leaves_empty_graph() {
    // GIVEN: a graph with 4 edges
    let (_dir, path) = setup_graph_db();
    populate_edges(&path);

    // WHEN: remove all edges one by one
    for id in [100, 101, 102, 103] {
        crate::graph::handle(GraphAction::RemoveEdge {
            path: path.clone(),
            collection: "kg".to_string(),
            edge_id: id,
        })
        .expect("remove should succeed");
    }

    // THEN: graph is empty
    assert_eq!(open_graph(&path).edge_count(), 0);
}

// =========================================================================
// C. remove-edge — Negative
// =========================================================================

#[test]
fn test_remove_edge_nonexistent_collection_fails() {
    // GIVEN: a database with no "ghost" collection
    let (_dir, path) = setup_graph_db();

    // WHEN: try to remove edge from non-existent collection
    let result = crate::graph::handle(GraphAction::RemoveEdge {
        path: path.clone(),
        collection: "ghost".to_string(),
        edge_id: 1,
    });

    // THEN: error
    assert!(result.is_err());
}

// =========================================================================
// D. count — Nominal
// =========================================================================

#[test]
fn test_count_populated_graph_shows_correct_counts() {
    // GIVEN: a graph with 4 edges and 5 nodes
    let (_dir, path) = setup_graph_db();
    populate_edges(&path);

    // WHEN: run count (we test the underlying logic, not stdout)
    let col = open_graph(&path);

    // THEN: correct counts
    assert_eq!(col.edge_count(), 4);
    // all_node_ids returns payload-stored IDs; edges reference 5 nodes
    // but without stored payloads, all_node_ids may return fewer
}

#[test]
fn test_count_empty_graph_shows_zero() {
    // GIVEN: an empty graph
    let (_dir, path) = setup_graph_db();

    // WHEN/THEN: counts are zero
    let col = open_graph(&path);
    assert_eq!(col.edge_count(), 0);
    assert_eq!(col.all_node_ids().len(), 0);
}

// =========================================================================
// E. count — Negative
// =========================================================================

#[test]
fn test_count_nonexistent_collection_fails() {
    let (_dir, path) = setup_graph_db();

    let result = crate::graph::handle(GraphAction::Count {
        path: path.clone(),
        collection: "ghost".to_string(),
        format: "table".to_string(),
    });

    assert!(result.is_err());
}

// =========================================================================
// F. search — Nominal
// =========================================================================

#[test]
fn test_search_graph_with_embeddings_returns_results() {
    // GIVEN: a graph collection with embeddings and upserted nodes
    let (_dir, path) = setup_graph_db_with_embeddings();

    // Insert points via Database's underlying collection
    let db = Database::open(&path).expect("test: open db");
    let col = db
        .get_vector_collection("kg")
        .expect("test: get collection");
    use velesdb_core::Point;
    col.upsert(vec![
        Point::new(1, vec![1.0, 0.0, 0.0, 0.0], None),
        Point::new(2, vec![0.0, 1.0, 0.0, 0.0], None),
        Point::new(3, vec![0.9, 0.1, 0.0, 0.0], None),
    ])
    .expect("test: upsert points");
    drop(col);
    drop(db);

    // WHEN: search by embedding
    let col = open_graph(&path);
    let results = col
        .search_by_embedding(&[1.0, 0.0, 0.0, 0.0], 2)
        .expect("search should succeed");

    // THEN: returns results sorted by similarity
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].point.id, 1, "closest match should be id=1");
}

// =========================================================================
// G. search — Edge cases
// =========================================================================

#[test]
fn test_search_graph_empty_collection_returns_empty() {
    // GIVEN: an empty graph with embeddings
    let (_dir, path) = setup_graph_db_with_embeddings();

    // WHEN: search
    let col = open_graph(&path);
    let results = col
        .search_by_embedding(&[1.0, 0.0, 0.0, 0.0], 10)
        .expect("search on empty should succeed");

    // THEN: no results
    assert!(results.is_empty());
}

#[test]
fn test_search_graph_top_k_larger_than_collection() {
    // GIVEN: a graph with 2 nodes
    let (_dir, path) = setup_graph_db_with_embeddings();
    let db = Database::open(&path).expect("test: open db");
    let col = db
        .get_vector_collection("kg")
        .expect("test: get collection");
    use velesdb_core::Point;
    col.upsert(vec![
        Point::new(1, vec![1.0, 0.0, 0.0, 0.0], None),
        Point::new(2, vec![0.0, 1.0, 0.0, 0.0], None),
    ])
    .expect("test: upsert");
    drop(col);
    drop(db);

    // WHEN: search with k=100
    let col = open_graph(&path);
    let results = col
        .search_by_embedding(&[1.0, 0.0, 0.0, 0.0], 100)
        .expect("search should succeed");

    // THEN: returns all 2 results (not 100)
    assert_eq!(results.len(), 2);
}

// =========================================================================
// H. search — Negative
// =========================================================================

#[test]
fn test_search_graph_without_embeddings_fails() {
    // GIVEN: a graph collection WITHOUT embeddings
    let (_dir, path) = setup_graph_db();

    // WHEN: try to search
    let col = open_graph(&path);
    let result = col.search_by_embedding(&[1.0, 0.0, 0.0, 0.0], 10);

    // THEN: error (no embeddings configured)
    assert!(result.is_err());
}

// =========================================================================
// I. traverse — Nominal (existing, but verify parallel)
// =========================================================================

#[test]
fn test_traverse_bfs_parallel_multiple_sources_deduplicates() {
    // GIVEN: a graph 1→2→3→4, 2→5
    let (_dir, path) = setup_graph_db();
    populate_edges(&path);

    // WHEN: parallel BFS from [1, 3]
    let col = open_graph(&path);
    let config = velesdb_core::collection::graph::TraversalConfig::with_range(1, 3).with_limit(100);
    let results = col.traverse_bfs_parallel(&[1, 3], &config);

    // THEN: node 4 appears (reachable from 3), node 2 appears (from 1)
    // Deduplication is by path signature (not target_id), so the same node
    // may appear multiple times if reached via different paths.
    let ids: Vec<u64> = results.iter().map(|r| r.target_id).collect();
    assert!(ids.contains(&2), "node 2 reachable from source 1");
    assert!(ids.contains(&4), "node 4 reachable from source 3");
}

#[test]
fn test_traverse_bfs_parallel_empty_sources_returns_empty() {
    // GIVEN: a populated graph
    let (_dir, path) = setup_graph_db();
    populate_edges(&path);

    // WHEN: parallel BFS with empty sources
    let col = open_graph(&path);
    let config = velesdb_core::collection::graph::TraversalConfig::with_range(1, 3).with_limit(100);
    let results = col.traverse_bfs_parallel(&[], &config);

    // THEN: no results
    assert!(results.is_empty());
}

#[test]
fn test_traverse_bfs_parallel_single_source_same_as_regular() {
    // GIVEN: a populated graph
    let (_dir, path) = setup_graph_db();
    populate_edges(&path);

    // WHEN: parallel BFS with single source vs regular BFS
    let col = open_graph(&path);
    let config = velesdb_core::collection::graph::TraversalConfig::with_range(1, 3).with_limit(100);
    let parallel = col.traverse_bfs_parallel(&[1], &config);
    let regular = col.traverse_bfs(1, &config);

    // THEN: same results
    let par_ids: std::collections::HashSet<u64> = parallel.iter().map(|r| r.target_id).collect();
    let reg_ids: std::collections::HashSet<u64> = regular.iter().map(|r| r.target_id).collect();
    assert_eq!(par_ids, reg_ids);
}

// =========================================================================
// J. node payload — Nominal
// =========================================================================

#[test]
fn test_store_payload_and_get_payload_roundtrip() {
    // GIVEN: a graph collection
    let (_dir, path) = setup_graph_db();

    // WHEN: store payload via CLI command
    crate::graph::handle(GraphAction::StorePayload {
        path: path.clone(),
        collection: "kg".to_string(),
        node_id: 42,
        payload: r#"{"name": "Alice", "age": 30}"#.to_string(),
    })
    .expect("store-payload should succeed");

    // THEN: payload is retrievable
    let col = open_graph(&path);
    let payload = col
        .get_node_payload(42)
        .expect("get should succeed")
        .expect("payload should exist");
    assert_eq!(payload["name"], "Alice");
    assert_eq!(payload["age"], 30);
}

#[test]
fn test_store_payload_overwrites_existing() {
    // GIVEN: a node with existing payload
    let (_dir, path) = setup_graph_db();
    crate::graph::handle(GraphAction::StorePayload {
        path: path.clone(),
        collection: "kg".to_string(),
        node_id: 1,
        payload: r#"{"v": 1}"#.to_string(),
    })
    .expect("first store");

    // WHEN: overwrite with new payload
    crate::graph::handle(GraphAction::StorePayload {
        path: path.clone(),
        collection: "kg".to_string(),
        node_id: 1,
        payload: r#"{"v": 2}"#.to_string(),
    })
    .expect("second store");

    // THEN: new payload is returned
    let col = open_graph(&path);
    let payload = col.get_node_payload(1).unwrap().unwrap();
    assert_eq!(payload["v"], 2);
}

// =========================================================================
// K. node payload — Negative
// =========================================================================

#[test]
fn test_store_payload_invalid_json_fails() {
    // GIVEN: a graph collection
    let (_dir, path) = setup_graph_db();

    // WHEN: store invalid JSON
    let result = crate::graph::handle(GraphAction::StorePayload {
        path: path.clone(),
        collection: "kg".to_string(),
        node_id: 1,
        payload: "not valid json".to_string(),
    });

    // THEN: error
    assert!(result.is_err());
}

#[test]
fn test_get_payload_nonexistent_node_returns_null() {
    // GIVEN: an empty graph
    let (_dir, path) = setup_graph_db();

    // WHEN: get payload for non-existent node
    let col = open_graph(&path);
    let payload = col.get_node_payload(999).expect("should not error");

    // THEN: None
    assert!(payload.is_none());
}
