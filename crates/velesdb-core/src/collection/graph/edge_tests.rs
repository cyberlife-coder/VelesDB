//! Tests for GraphEdge and EdgeStore.
//!
//! TDD: Tests written BEFORE implementation (RED phase).

use super::*;
use serde_json::json;
use std::collections::HashMap;

// =============================================================================
// AC-1: Edge creation and properties
// =============================================================================

#[test]
fn test_create_edge_basic() {
    let edge = GraphEdge::new(1, 100, 200, "MENTIONS");

    assert_eq!(edge.id(), 1);
    assert_eq!(edge.source(), 100);
    assert_eq!(edge.target(), 200);
    assert_eq!(edge.label(), "MENTIONS");
    assert!(edge.properties().is_empty());
}

#[test]
fn test_edge_with_properties() {
    let mut props = HashMap::new();
    props.insert("date".to_string(), json!("2026-01-15"));
    props.insert("role".to_string(), json!("author"));

    let edge = GraphEdge::new(1, 10, 20, "WROTE").with_properties(props);

    assert_eq!(edge.properties().len(), 2);
    assert_eq!(edge.property("date"), Some(&json!("2026-01-15")));
    assert_eq!(edge.property("role"), Some(&json!("author")));
}

#[test]
fn test_edge_serialization_roundtrip() {
    let mut props = HashMap::new();
    props.insert("weight".to_string(), json!(0.95));

    let edge = GraphEdge::new(42, 1, 2, "RELATED").with_properties(props);

    let json = serde_json::to_string(&edge).expect("serialization failed");
    let deserialized: GraphEdge = serde_json::from_str(&json).expect("deserialization failed");

    assert_eq!(edge.id(), deserialized.id());
    assert_eq!(edge.source(), deserialized.source());
    assert_eq!(edge.target(), deserialized.target());
    assert_eq!(edge.label(), deserialized.label());
}

// =============================================================================
// AC-2: EdgeStore - add and get neighbors
// =============================================================================

#[test]
fn test_edge_store_add_edge() {
    let mut store = EdgeStore::new();
    let edge = GraphEdge::new(1, 100, 200, "KNOWS");

    store.add_edge(edge);

    assert_eq!(store.edge_count(), 1);
}

#[test]
fn test_edge_store_get_outgoing() {
    let mut store = EdgeStore::new();
    store.add_edge(GraphEdge::new(1, 100, 200, "KNOWS"));
    store.add_edge(GraphEdge::new(2, 100, 300, "WORKS_AT"));

    let outgoing = store.get_outgoing(100);
    assert_eq!(outgoing.len(), 2);

    // Node 200 has no outgoing edges
    let outgoing_200 = store.get_outgoing(200);
    assert!(outgoing_200.is_empty());
}

#[test]
fn test_edge_store_get_incoming() {
    let mut store = EdgeStore::new();
    store.add_edge(GraphEdge::new(1, 100, 200, "KNOWS"));
    store.add_edge(GraphEdge::new(2, 300, 200, "KNOWS"));

    let incoming = store.get_incoming(200);
    assert_eq!(incoming.len(), 2);

    // Node 100 has no incoming edges
    let incoming_100 = store.get_incoming(100);
    assert!(incoming_100.is_empty());
}

#[test]
fn test_edge_store_bidirectional_traversal() {
    let mut store = EdgeStore::new();
    store.add_edge(GraphEdge::new(1, 10, 20, "RELATED"));

    // Can traverse from source
    let from_source = store.get_outgoing(10);
    assert_eq!(from_source.len(), 1);
    assert_eq!(from_source[0].target(), 20);

    // Can traverse from target
    let from_target = store.get_incoming(20);
    assert_eq!(from_target.len(), 1);
    assert_eq!(from_target[0].source(), 10);
}

// =============================================================================
// AC-3: Cascade delete
// =============================================================================

#[test]
fn test_edge_store_remove_edge() {
    let mut store = EdgeStore::new();
    store.add_edge(GraphEdge::new(1, 100, 200, "KNOWS"));
    store.add_edge(GraphEdge::new(2, 100, 300, "WORKS_AT"));

    assert_eq!(store.edge_count(), 2);

    store.remove_edge(1);

    assert_eq!(store.edge_count(), 1);
    assert!(store.get_outgoing(100).iter().all(|e| e.id() != 1));
}

#[test]
fn test_cascade_delete_removes_all_edges() {
    let mut store = EdgeStore::new();
    // Node 100 has 3 outgoing and 2 incoming edges
    store.add_edge(GraphEdge::new(1, 100, 200, "A"));
    store.add_edge(GraphEdge::new(2, 100, 300, "B"));
    store.add_edge(GraphEdge::new(3, 100, 400, "C"));
    store.add_edge(GraphEdge::new(4, 500, 100, "D"));
    store.add_edge(GraphEdge::new(5, 600, 100, "E"));

    assert_eq!(store.edge_count(), 5);

    store.remove_node_edges(100);

    assert_eq!(store.edge_count(), 0);
    assert!(store.get_outgoing(100).is_empty());
    assert!(store.get_incoming(100).is_empty());
}

#[test]
fn test_cascade_delete_preserves_other_edges() {
    let mut store = EdgeStore::new();
    store.add_edge(GraphEdge::new(1, 100, 200, "A"));
    store.add_edge(GraphEdge::new(2, 300, 400, "B")); // Unrelated edge

    store.remove_node_edges(100);

    assert_eq!(store.edge_count(), 1);
    assert!(!store.get_outgoing(300).is_empty());
}

// =============================================================================
// Edge filtering by label
// =============================================================================

#[test]
fn test_get_outgoing_by_label() {
    let mut store = EdgeStore::new();
    store.add_edge(GraphEdge::new(1, 100, 200, "KNOWS"));
    store.add_edge(GraphEdge::new(2, 100, 300, "WORKS_AT"));
    store.add_edge(GraphEdge::new(3, 100, 400, "KNOWS"));

    let knows_edges = store.get_outgoing_by_label(100, "KNOWS");
    assert_eq!(knows_edges.len(), 2);

    let works_at_edges = store.get_outgoing_by_label(100, "WORKS_AT");
    assert_eq!(works_at_edges.len(), 1);

    let none_edges = store.get_outgoing_by_label(100, "UNKNOWN");
    assert!(none_edges.is_empty());
}

// =============================================================================
// Get edge by ID
// =============================================================================

#[test]
fn test_get_edge_by_id() {
    let mut store = EdgeStore::new();
    store.add_edge(GraphEdge::new(42, 100, 200, "TEST"));

    let edge = store.get_edge(42);
    assert!(edge.is_some());
    assert_eq!(edge.unwrap().label(), "TEST");

    let missing = store.get_edge(999);
    assert!(missing.is_none());
}

// =============================================================================
// Edge cases
// =============================================================================

#[test]
fn test_empty_store() {
    let store = EdgeStore::new();

    assert_eq!(store.edge_count(), 0);
    assert!(store.get_outgoing(100).is_empty());
    assert!(store.get_incoming(100).is_empty());
    assert!(store.get_edge(1).is_none());
}

#[test]
fn test_self_loop_edge() {
    let mut store = EdgeStore::new();
    store.add_edge(GraphEdge::new(1, 100, 100, "SELF_REF"));

    // Self-loop appears in both outgoing and incoming
    assert_eq!(store.get_outgoing(100).len(), 1);
    assert_eq!(store.get_incoming(100).len(), 1);
}
