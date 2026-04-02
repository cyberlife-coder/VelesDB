//! BDD tests for `VelesQL` v3.5 Phase 5: SELECT EDGES and INSERT NODE.
//!
//! All tests exercise the full pipeline: SQL string -> parse -> execute -> verify.
//! Requires `persistence` feature (gated in `bdd.rs`).

use super::helpers::{create_test_db, execute_sql};

// =========================================================================
// Helper: create a graph collection and populate it with edges/nodes.
// =========================================================================

fn setup_graph_collection(db: &velesdb_core::Database) {
    execute_sql(
        db,
        "CREATE GRAPH COLLECTION kg (dimension = 4, metric = 'cosine') SCHEMALESS;",
    )
    .expect("test: CREATE GRAPH COLLECTION should succeed");
}

fn populate_graph_edges(db: &velesdb_core::Database) {
    execute_sql(
        db,
        "INSERT EDGE INTO kg (source = 1, target = 2, label = 'KNOWS');",
    )
    .expect("test: INSERT EDGE 1->2 KNOWS");
    execute_sql(
        db,
        "INSERT EDGE INTO kg (source = 1, target = 3, label = 'WORKS_WITH');",
    )
    .expect("test: INSERT EDGE 1->3 WORKS_WITH");
    execute_sql(
        db,
        "INSERT EDGE INTO kg (source = 2, target = 3, label = 'KNOWS');",
    )
    .expect("test: INSERT EDGE 2->3 KNOWS");
}

// =========================================================================
// A. SELECT EDGES — nominal
// =========================================================================

#[test]
fn test_select_all_edges() {
    let (_dir, db) = create_test_db();
    setup_graph_collection(&db);
    populate_graph_edges(&db);

    let results = execute_sql(&db, "SELECT EDGES FROM kg;").expect("SELECT EDGES should work");
    assert_eq!(results.len(), 3, "Graph has 3 edges");
}

#[test]
fn test_select_edges_by_source() {
    let (_dir, db) = create_test_db();
    setup_graph_collection(&db);
    populate_graph_edges(&db);

    let results =
        execute_sql(&db, "SELECT EDGES FROM kg WHERE source = 1;").expect("filter by source");
    assert_eq!(results.len(), 2, "Node 1 has 2 outgoing edges");
}

#[test]
fn test_select_edges_by_label() {
    let (_dir, db) = create_test_db();
    setup_graph_collection(&db);
    populate_graph_edges(&db);

    let results =
        execute_sql(&db, "SELECT EDGES FROM kg WHERE label = 'KNOWS';").expect("filter by label");
    assert_eq!(results.len(), 2, "Two KNOWS edges exist");
}

// =========================================================================
// B. INSERT NODE — nominal
// =========================================================================

#[test]
fn test_insert_node_creates_payload() {
    let (_dir, db) = create_test_db();
    setup_graph_collection(&db);

    execute_sql(
        &db,
        "INSERT NODE INTO kg (id = 42, payload = '{\"name\": \"Alice\"}');",
    )
    .expect("INSERT NODE should succeed");

    // Verify via API.
    let graph = db
        .get_graph_collection("kg")
        .expect("kg should be a graph collection");
    let payload = graph
        .get_node_payload(42)
        .expect("payload retrieval should succeed")
        .expect("payload should exist");
    assert_eq!(payload["name"], "Alice");
}

// =========================================================================
// C. Edge cases
// =========================================================================

#[test]
fn test_select_edges_empty_graph() {
    let (_dir, db) = create_test_db();
    setup_graph_collection(&db);

    let results = execute_sql(&db, "SELECT EDGES FROM kg;").expect("empty graph should work");
    assert_eq!(results.len(), 0, "No edges in empty graph");
}

#[test]
fn test_select_edges_with_limit() {
    let (_dir, db) = create_test_db();
    setup_graph_collection(&db);
    populate_graph_edges(&db);

    let results =
        execute_sql(&db, "SELECT EDGES FROM kg LIMIT 2;").expect("LIMIT should be respected");
    assert_eq!(results.len(), 2, "LIMIT 2 should return at most 2");
}

#[test]
fn test_insert_node_updates_existing() {
    let (_dir, db) = create_test_db();
    setup_graph_collection(&db);

    execute_sql(
        &db,
        "INSERT NODE INTO kg (id = 10, payload = '{\"name\": \"Bob\"}');",
    )
    .expect("first INSERT NODE");

    execute_sql(
        &db,
        "INSERT NODE INTO kg (id = 10, payload = '{\"name\": \"Robert\"}');",
    )
    .expect("second INSERT NODE should update");

    let graph = db
        .get_graph_collection("kg")
        .expect("kg should be a graph collection");
    let payload = graph
        .get_node_payload(10)
        .expect("retrieval should succeed")
        .expect("payload should exist");
    assert_eq!(payload["name"], "Robert", "Payload should be updated");
}

// =========================================================================
// D. Complex lifecycle
// =========================================================================

#[test]
fn test_graph_node_edge_lifecycle() {
    let (_dir, db) = create_test_db();
    setup_graph_collection(&db);

    // Insert nodes.
    execute_sql(
        &db,
        "INSERT NODE INTO kg (id = 1, payload = '{\"name\": \"Alice\"}');",
    )
    .expect("insert node 1");
    execute_sql(
        &db,
        "INSERT NODE INTO kg (id = 2, payload = '{\"name\": \"Bob\"}');",
    )
    .expect("insert node 2");

    // Insert edge.
    execute_sql(
        &db,
        "INSERT EDGE INTO kg (source = 1, target = 2, label = 'KNOWS');",
    )
    .expect("insert edge");

    // SELECT EDGES should show 1 edge.
    let results = execute_sql(&db, "SELECT EDGES FROM kg;").expect("select edges");
    assert_eq!(results.len(), 1);

    // Delete the edge (use a known auto-generated ID).
    let edge_id = results[0].point.id;
    let delete_sql = format!("DELETE EDGE {edge_id} FROM kg;");
    execute_sql(&db, &delete_sql).expect("delete edge");

    // SELECT EDGES should now return 0.
    let results_after = execute_sql(&db, "SELECT EDGES FROM kg;").expect("select after delete");
    assert_eq!(results_after.len(), 0);
}

// =========================================================================
// E. Negative
// =========================================================================

#[test]
fn test_select_edges_from_vector_collection_fails() {
    let (_dir, db) = create_test_db();

    // Create a vector collection (not a graph).
    execute_sql(
        &db,
        "CREATE COLLECTION docs (dimension = 4, metric = 'cosine');",
    )
    .expect("create vector collection");

    let err = execute_sql(&db, "SELECT EDGES FROM docs;").expect_err("should fail on non-graph");
    let msg = format!("{err}");
    assert!(
        msg.contains("not found") || msg.contains("NotFound"),
        "Error should indicate collection not found as graph: {msg}"
    );
}

// =========================================================================
// F. Label index consistency (Resolution 3)
// =========================================================================

/// GIVEN a graph collection with nodes labeled `["Person"]`
/// WHEN a MATCH query filters by label `(n:Person)`
/// THEN the label index returns exactly the labeled nodes.
#[test]
fn test_label_index_consistency_after_upsert() {
    use std::collections::HashMap;
    use velesdb_core::velesql;

    let (_dir, db) = create_test_db();
    setup_graph_collection(&db);

    // Insert labeled nodes via the graph collection API.
    let gc = db
        .get_graph_collection("kg")
        .expect("test: kg should exist as graph collection");

    gc.upsert_node_payload(
        1,
        &serde_json::json!({"_labels": ["Person"], "name": "Alice"}),
    )
    .expect("test: upsert node 1");
    gc.upsert_node_payload(
        2,
        &serde_json::json!({"_labels": ["Person"], "name": "Bob"}),
    )
    .expect("test: upsert node 2");
    gc.upsert_node_payload(
        3,
        &serde_json::json!({"_labels": ["Company"], "name": "Acme"}),
    )
    .expect("test: upsert node 3");

    // WHEN: execute a MATCH query filtering by Person label.
    let match_clause = velesql::MatchClause {
        patterns: vec![velesql::GraphPattern {
            name: None,
            nodes: vec![
                velesql::NodePattern::new()
                    .with_alias("p")
                    .with_label("Person"),
                velesql::NodePattern::new().with_alias("target"),
            ],
            relationships: vec![velesql::RelationshipPattern::new(
                velesql::Direction::Outgoing,
            )],
        }],
        where_clause: None,
        return_clause: velesql::ReturnClause {
            items: vec![velesql::ReturnItem {
                expression: "*".to_string(),
                alias: None,
            }],
            order_by: None,
            limit: Some(100),
        },
    };

    // Add an edge from Person 1 to Person 2 so traversal produces results.
    let edge =
        velesdb_core::GraphEdge::new(1, 1, 2, "KNOWS").expect("test: create edge 1->2 KNOWS");
    gc.add_edge(edge).expect("test: add edge");

    let results = gc
        .execute_match(&match_clause, &HashMap::new())
        .expect("test: MATCH (p:Person) should succeed");

    // THEN: only Person-labeled start nodes produce results.
    // Node 1 (Person) is a start node, traverses to node 2 via KNOWS.
    assert!(
        !results.is_empty(),
        "MATCH (p:Person) should find at least one path"
    );

    // Verify that the start node binding "p" is always a Person node (1 or 2).
    for r in &results {
        if let Some(&start_id) = r.bindings.get("p") {
            assert!(
                start_id == 1 || start_id == 2,
                "Start node binding 'p' should be a Person (1 or 2), got {}",
                start_id
            );
        }
    }
}

/// GIVEN a graph collection with labeled nodes
/// WHEN a node is deleted
/// THEN MATCH queries no longer return the deleted node as a start node.
#[test]
fn test_label_index_consistency_after_delete() {
    use std::collections::HashMap;
    use velesdb_core::velesql;

    let (_dir, db) = create_test_db();
    setup_graph_collection(&db);

    let gc = db
        .get_graph_collection("kg")
        .expect("test: kg should exist");

    // Insert two Person nodes and an edge.
    gc.upsert_node_payload(
        10,
        &serde_json::json!({"_labels": ["Person"], "name": "Carol"}),
    )
    .expect("test: upsert node 10");
    gc.upsert_node_payload(
        20,
        &serde_json::json!({"_labels": ["Person"], "name": "Dave"}),
    )
    .expect("test: upsert node 20");

    let edge = velesdb_core::GraphEdge::new(1, 10, 20, "KNOWS").expect("test: edge 10->20");
    gc.add_edge(edge).expect("test: add edge");

    // Build MATCH clause filtering by Person label.
    let match_clause = velesql::MatchClause {
        patterns: vec![velesql::GraphPattern {
            name: None,
            nodes: vec![
                velesql::NodePattern::new()
                    .with_alias("p")
                    .with_label("Person"),
                velesql::NodePattern::new().with_alias("t"),
            ],
            relationships: vec![velesql::RelationshipPattern::new(
                velesql::Direction::Outgoing,
            )],
        }],
        where_clause: None,
        return_clause: velesql::ReturnClause {
            items: vec![velesql::ReturnItem {
                expression: "*".to_string(),
                alias: None,
            }],
            order_by: None,
            limit: Some(100),
        },
    };

    // Verify the path exists before deletion.
    let before = gc
        .execute_match(&match_clause, &HashMap::new())
        .expect("test: MATCH before delete");
    assert!(
        before.iter().any(|r| r.bindings.get("p") == Some(&10)),
        "Node 10 should be a start node before deletion"
    );

    // WHEN: delete node 10.
    gc.delete(&[10]).expect("test: delete node 10");

    // THEN: MATCH no longer returns node 10 as a start node.
    let after = gc
        .execute_match(&match_clause, &HashMap::new())
        .expect("test: MATCH after delete");
    assert!(
        !after.iter().any(|r| r.bindings.get("p") == Some(&10)),
        "Deleted node 10 should not appear as start node after deletion"
    );
}

// =========================================================================
// G. Bidirectional traversal dedup (Resolution 2 — BDD level)
// =========================================================================

/// GIVEN a graph with A->B->C and C->B (bidirectional through B)
/// WHEN executing BFS in both directions from A
/// THEN each target node appears at most once in results.
#[test]
fn test_bfs_traverse_both_dedup_e2e() {
    let (_dir, db) = create_test_db();
    setup_graph_collection(&db);

    let gc = db
        .get_graph_collection("kg")
        .expect("test: kg should exist");

    // Build graph: A(1)->B(2)->C(3), C(3)->B(2) (reverse).
    gc.upsert_node_payload(1, &serde_json::json!({"name": "A"}))
        .expect("test: node A");
    gc.upsert_node_payload(2, &serde_json::json!({"name": "B"}))
        .expect("test: node B");
    gc.upsert_node_payload(3, &serde_json::json!({"name": "C"}))
        .expect("test: node C");

    let e1 = velesdb_core::GraphEdge::new(1, 1, 2, "LINK").expect("test: A->B");
    gc.add_edge(e1).expect("test: add A->B");
    let e2 = velesdb_core::GraphEdge::new(2, 2, 3, "LINK").expect("test: B->C");
    gc.add_edge(e2).expect("test: add B->C");
    let e3 = velesdb_core::GraphEdge::new(3, 3, 2, "LINK").expect("test: C->B");
    gc.add_edge(e3).expect("test: add C->B");

    // Use the traversal API directly.
    let config = velesdb_core::TraversalConfig::with_range(1, 2).with_limit(100);
    let results = gc.traverse_bfs(1, &config);

    // Verify no duplicate target IDs.
    let mut seen = std::collections::HashSet::new();
    for r in &results {
        assert!(
            seen.insert(r.target_id),
            "Node {} appeared more than once in BFS results",
            r.target_id
        );
    }

    // Expected: node 2 (depth 1) and node 3 (depth 2).
    assert!(
        results.iter().any(|r| r.target_id == 2),
        "Node B (2) should be reachable"
    );
    assert!(
        results.iter().any(|r| r.target_id == 3),
        "Node C (3) should be reachable"
    );
}

// =========================================================================
// H. Concurrent edge add + query safety (Resolution 4)
// =========================================================================

/// GIVEN a graph collection
/// WHEN one thread adds edges while another queries
/// THEN queries never panic and never return corrupted data.
#[test]
fn test_concurrent_edge_add_and_query() {
    use std::sync::Arc;

    let (_dir, db) = create_test_db();
    setup_graph_collection(&db);

    let gc = Arc::new(
        db.get_graph_collection("kg")
            .expect("test: kg should exist"),
    );

    // Pre-insert some nodes.
    for id in 1..=10u64 {
        gc.upsert_node_payload(id, &serde_json::json!({"name": format!("node_{id}")}))
            .expect("test: upsert node");
    }

    // Writer thread: adds edges concurrently.
    let gc_writer = Arc::clone(&gc);
    let writer = std::thread::spawn(move || {
        for i in 1..=50u64 {
            let source = (i % 10) + 1;
            let target = ((i + 3) % 10) + 1;
            if source != target {
                let edge = velesdb_core::GraphEdge::new(i, source, target, "CONNECTS")
                    .expect("test: create edge");
                // Ignore duplicates (add_edge may error if edge ID already exists).
                let _ = gc_writer.add_edge(edge);
            }
        }
    });

    // Reader thread: queries concurrently.
    let gc_reader = Arc::clone(&gc);
    let reader = std::thread::spawn(move || {
        let mut success_count = 0u32;
        for _ in 0..50 {
            let edges = gc_reader.get_edges(None);
            // Verify no corrupted data: all edges must have valid labels.
            for e in &edges {
                assert!(
                    !e.label().is_empty(),
                    "Edge {} has an empty label (corruption)",
                    e.id()
                );
            }
            success_count += 1;
        }
        success_count
    });

    writer.join().expect("test: writer thread should not panic");
    let reads = reader.join().expect("test: reader thread should not panic");
    assert_eq!(reads, 50, "All 50 reads should complete without panic");

    // Final consistency: all edges are valid.
    let final_edges = gc.get_edges(None);
    for e in &final_edges {
        assert!(
            !e.label().is_empty(),
            "Edge {} has corrupted label after concurrent ops",
            e.id()
        );
    }
}
