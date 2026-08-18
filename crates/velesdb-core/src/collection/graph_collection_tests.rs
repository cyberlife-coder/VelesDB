use super::*;
use crate::collection::graph::GraphSchema;
use crate::distance::DistanceMetric;
use std::collections::HashMap;
use tempfile::{tempdir, TempDir};

/// Creates a schemaless cosine `GraphCollection` in a fresh temp dir.
///
/// Returns the `TempDir` guard alongside the collection so the backing
/// directory outlives the test. `dimension` controls embedding support
/// (`None` for payload/edge-only collections, `Some(n)` for searchable ones).
fn make_test_collection(dimension: Option<usize>) -> (TempDir, GraphCollection) {
    let dir = tempdir().unwrap();
    let col = GraphCollection::create(
        dir.path().to_path_buf(),
        "kg",
        dimension,
        DistanceMetric::Cosine,
        GraphSchema::schemaless(),
    )
    .unwrap();
    (dir, col)
}

#[test]
fn test_all_node_ids_returns_ids_with_payload() {
    let (_dir, col) = make_test_collection(None);

    // Store payloads on two nodes
    col.upsert_node_payload(10, &serde_json::json!({"name": "Alice"}))
        .unwrap();
    col.upsert_node_payload(20, &serde_json::json!({"name": "Bob"}))
        .unwrap();

    let ids = col.all_node_ids();
    assert!(ids.contains(&10), "node 10 should be present");
    assert!(ids.contains(&20), "node 20 should be present");
    assert_eq!(ids.len(), 2);
}

#[test]
fn test_upsert_node_with_embedding_is_searchable() {
    let (_dir, col) = make_test_collection(Some(4));

    col.upsert_node(
        10,
        &serde_json::json!({"name": "Alice"}),
        Some(vec![1.0, 0.0, 0.0, 0.0]),
    )
    .unwrap();

    assert_eq!(
        col.get_node_payload(10).unwrap(),
        Some(serde_json::json!({"name": "Alice"}))
    );
    let results = col.search_by_embedding(&[1.0, 0.0, 0.0, 0.0], 1).unwrap();
    assert_eq!(results[0].point.id, 10);
}

#[test]
fn test_edge_count_returns_correct_count() {
    let (_dir, col) = make_test_collection(None);

    assert_eq!(col.edge_count(), 0);
    for id in [10, 20, 30] {
        col.upsert_node_payload(id, &serde_json::json!({})).unwrap();
    }

    let edge1 = crate::collection::graph::GraphEdge::new(1, 10, 20, "knows").unwrap();
    col.add_edge(edge1).unwrap();
    assert_eq!(col.edge_count(), 1);

    let edge2 = crate::collection::graph::GraphEdge::new(2, 20, 30, "likes").unwrap();
    col.add_edge(edge2).unwrap();
    assert_eq!(col.edge_count(), 2);
}

#[test]
fn test_traverse_bfs_parallel_through_graph_collection() {
    let (_dir, col) = make_test_collection(None);

    // Build chain: 1->2->3
    for id in [1, 2, 3] {
        col.upsert_node_payload(id, &serde_json::json!({})).unwrap();
    }
    col.add_edge(GraphEdge::new(1, 1, 2, "NEXT").unwrap())
        .unwrap();
    col.add_edge(GraphEdge::new(2, 2, 3, "NEXT").unwrap())
        .unwrap();

    let config = TraversalConfig {
        max_depth: 3,
        min_depth: 1,
        ..TraversalConfig::default()
    };
    let results = col.traverse_bfs_parallel(&[1], &config);
    let target_ids: std::collections::HashSet<u64> = results.iter().map(|r| r.target_id).collect();
    assert!(target_ids.contains(&2), "should reach node 2");
    assert!(target_ids.contains(&3), "should reach node 3");
}

#[test]
fn test_execute_match_finds_edges() {
    let (_dir, col) = make_test_collection(None);

    // Store node payloads with labels
    col.upsert_node_payload(
        10,
        &serde_json::json!({"_labels": ["Person"], "name": "Alice"}),
    )
    .unwrap();
    col.upsert_node_payload(
        20,
        &serde_json::json!({"_labels": ["Person"], "name": "Bob"}),
    )
    .unwrap();

    // Add edge: Alice -> Bob
    let edge = crate::collection::graph::GraphEdge::new(1, 10, 20, "KNOWS").unwrap();
    col.add_edge(edge).unwrap();

    // MATCH query through the GraphCollection delegate
    let match_clause = crate::velesql::MatchClause {
        patterns: vec![crate::velesql::GraphPattern {
            name: None,
            nodes: vec![
                crate::velesql::NodePattern::new().with_alias("a"),
                crate::velesql::NodePattern::new().with_alias("b"),
            ],
            relationships: vec![crate::velesql::RelationshipPattern::new(
                crate::velesql::Direction::Outgoing,
            )],
        }],
        where_clause: None,
        return_clause: crate::velesql::ReturnClause {
            items: vec![],
            order_by: None,
            limit: Some(10),
        },
    };

    let params = HashMap::new();
    let results = col.execute_match(&match_clause, &params).unwrap();
    assert!(
        !results.is_empty(),
        "execute_match should find the KNOWS edge"
    );
    assert_eq!(results[0].node_id, 20, "target should be Bob (id=20)");
}

#[test]
fn test_has_edge_and_remove_edge() {
    let (_dir, col) = make_test_collection(None);
    assert!(!col.has_edge(7), "unknown edge id is absent");
    for id in [10, 20] {
        col.upsert_node_payload(id, &serde_json::json!({})).unwrap();
    }

    col.add_edge(GraphEdge::new(7, 10, 20, "KNOWS").unwrap())
        .unwrap();
    assert!(col.has_edge(7), "edge present after add");

    assert!(col.remove_edge(7), "removing an existing edge returns true");
    assert!(!col.has_edge(7), "edge gone after remove");
    assert!(!col.remove_edge(7), "removing a missing edge returns false");
}

#[test]
fn test_upsert_node_without_vector_stores_payload_only() {
    // No embeddings: the `None`-vector branch delegates to upsert_node_payload.
    let (_dir, col) = make_test_collection(None);
    col.upsert_node(42, &serde_json::json!({"name": "Carol"}), None)
        .unwrap();
    assert_eq!(
        col.get_node_payload(42).unwrap(),
        Some(serde_json::json!({"name": "Carol"}))
    );
    assert!(col.all_node_ids().contains(&42));
    assert!(!col.has_embeddings(), "no embeddings without a dimension");
}

#[test]
fn test_get_edges_filtered_by_label() {
    let (_dir, col) = make_test_collection(None);
    for id in [10, 20, 30, 40] {
        col.upsert_node_payload(id, &serde_json::json!({})).unwrap();
    }
    col.add_edge(GraphEdge::new(1, 10, 20, "KNOWS").unwrap())
        .unwrap();
    col.add_edge(GraphEdge::new(2, 20, 30, "LIKES").unwrap())
        .unwrap();
    col.add_edge(GraphEdge::new(3, 30, 40, "KNOWS").unwrap())
        .unwrap();

    let knows = col.get_edges(Some("KNOWS"));
    assert_eq!(knows.len(), 2, "two KNOWS edges");
    assert!(knows.iter().all(|e| e.label() == "KNOWS"));

    let all = col.get_edges(None);
    assert_eq!(all.len(), 3, "three edges total");
}

#[test]
fn test_node_degree_and_directional_edges() {
    let (_dir, col) = make_test_collection(None);
    for id in [10, 20, 30, 40] {
        col.upsert_node_payload(id, &serde_json::json!({})).unwrap();
    }
    col.add_edge(GraphEdge::new(1, 10, 20, "NEXT").unwrap())
        .unwrap();
    col.add_edge(GraphEdge::new(2, 30, 20, "NEXT").unwrap())
        .unwrap();
    col.add_edge(GraphEdge::new(3, 20, 40, "NEXT").unwrap())
        .unwrap();

    // Node 20: 2 incoming (from 10, 30), 1 outgoing (to 40).
    assert_eq!(col.node_degree(20), (2, 1));
    assert_eq!(col.get_incoming(20).len(), 2);
    let outgoing = col.get_outgoing(20);
    assert_eq!(outgoing.len(), 1);
    assert_eq!(outgoing[0].target(), 40);
}

#[test]
fn test_delete_removes_node_payload() {
    let (_dir, col) = make_test_collection(None);
    col.upsert_node_payload(10, &serde_json::json!({"k": 1}))
        .unwrap();
    col.upsert_node_payload(20, &serde_json::json!({"k": 2}))
        .unwrap();
    assert_eq!(col.all_node_ids().len(), 2);

    col.delete(&[10]).unwrap();
    assert!(col.get(&[10])[0].is_none(), "deleted node is gone");
    assert!(
        !col.all_node_ids().contains(&10),
        "deleted node leaves the id set"
    );
    assert!(col.get_node_payload(20).unwrap().is_some(), "node 20 stays");
}

#[test]
fn test_scroll_batch_paginates_embedded_nodes() {
    // scroll_batch iterates the point (vector) store, so use embeddings.
    let (_dir, col) = make_test_collection(Some(2));
    for id in [1u64, 2, 3] {
        col.upsert_node(id, &serde_json::json!({"id": id}), Some(vec![1.0, 0.0]))
            .unwrap();
    }
    assert!(!col.is_empty());
    assert_eq!(col.len(), 3);

    let first = col.scroll_batch(None, 2, None).unwrap();
    assert_eq!(first.points.len(), 2, "first page has 2 of 3 nodes");
    let cursor = first.next_cursor.expect("non-empty page yields a cursor");
    let second = col.scroll_batch(Some(cursor), 2, None).unwrap();
    assert_eq!(second.points.len(), 1, "second page has the last node");
    // A page past the end returns no points (and therefore no cursor).
    let tail_cursor = second.next_cursor.expect("page yields a cursor");
    let third = col.scroll_batch(Some(tail_cursor), 2, None).unwrap();
    assert!(third.points.is_empty(), "no points past the end");
    assert!(third.next_cursor.is_none(), "empty page yields no cursor");

    // batch_size 0 is rejected.
    assert!(col.scroll_batch(None, 0, None).is_err());
}

#[test]
fn test_flush_and_flush_full_succeed() {
    let (_dir, col) = make_test_collection(None);
    col.upsert_node_payload(1, &serde_json::json!({"k": 1}))
        .unwrap();
    col.upsert_node_payload(2, &serde_json::json!({})).unwrap();
    col.add_edge(GraphEdge::new(1, 1, 2, "NEXT").unwrap())
        .unwrap();
    col.flush().expect("fast-path flush succeeds");
    col.flush_full().expect("full durability flush succeeds");
}

#[test]
fn test_reopen_recovers_edges_and_payloads() {
    let dir = tempdir().unwrap();
    let path = dir.path().to_path_buf();
    {
        let col = GraphCollection::create(
            path.clone(),
            "kg",
            None,
            DistanceMetric::Cosine,
            GraphSchema::schemaless(),
        )
        .unwrap();
        col.upsert_node_payload(1, &serde_json::json!({"name": "A"}))
            .unwrap();
        col.upsert_node_payload(2, &serde_json::json!({})).unwrap();
        col.add_edge(GraphEdge::new(5, 1, 2, "NEXT").unwrap())
            .unwrap();
        col.flush_full().unwrap();
    }
    let reopened = GraphCollection::open(path).unwrap();
    assert_eq!(reopened.name(), "kg");
    assert!(reopened.has_edge(5), "edge survives reopen");
    assert_eq!(
        reopened.get_node_payload(1).unwrap(),
        Some(serde_json::json!({"name": "A"}))
    );
}
