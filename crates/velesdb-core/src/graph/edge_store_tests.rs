//! Tests for InMemoryEdgeStore.

use super::edge_store::InMemoryEdgeStore;
use super::types::{GraphEdge, GraphNode};

fn build_test_graph() -> InMemoryEdgeStore {
    let mut store = InMemoryEdgeStore::new();
    store.add_node(GraphNode::new(1, "Person")).unwrap();
    store.add_node(GraphNode::new(2, "Person")).unwrap();
    store.add_node(GraphNode::new(3, "Company")).unwrap();
    store
        .add_edge(GraphEdge::new(100, 1, 2, "KNOWS").unwrap())
        .unwrap();
    store
        .add_edge(GraphEdge::new(101, 1, 3, "WORKS_AT").unwrap())
        .unwrap();
    store
        .add_edge(GraphEdge::new(102, 2, 3, "WORKS_AT").unwrap())
        .unwrap();
    store
}

#[test]
fn test_add_and_get_node() {
    let mut store = InMemoryEdgeStore::new();
    store.add_node(GraphNode::new(1, "Person")).unwrap();
    assert!(store.has_node(1));
    assert!(!store.has_node(2));

    let node = store.get_node(1).unwrap();
    assert_eq!(node.label(), "Person");
}

#[test]
fn test_add_duplicate_node_fails() {
    let mut store = InMemoryEdgeStore::new();
    store.add_node(GraphNode::new(1, "Person")).unwrap();
    let result = store.add_node(GraphNode::new(1, "Company"));
    assert!(result.is_err());
}

#[test]
fn test_remove_node_cascades_edges() {
    let mut store = build_test_graph();
    assert_eq!(store.node_count(), 3);
    assert_eq!(store.edge_count(), 3);

    // Remove node 1 → should cascade-delete edges 100 and 101
    let removed = store.remove_node(1);
    assert!(removed.is_some());
    assert_eq!(store.node_count(), 2);
    assert_eq!(store.edge_count(), 1); // Only edge 102 (2→3) remains
    assert!(!store.has_edge(100));
    assert!(!store.has_edge(101));
    assert!(store.has_edge(102));
}

#[test]
fn test_add_and_get_edge() {
    let store = build_test_graph();
    let edge = store.get_edge(100).unwrap();
    assert_eq!(edge.source(), 1);
    assert_eq!(edge.target(), 2);
    assert_eq!(edge.label(), "KNOWS");
}

#[test]
fn test_add_duplicate_edge_fails() {
    let mut store = build_test_graph();
    let result = store.add_edge(GraphEdge::new(100, 2, 1, "LIKES").unwrap());
    assert!(result.is_err());
}

#[test]
fn test_get_outgoing() {
    let store = build_test_graph();
    let outgoing = store.get_outgoing(1);
    assert_eq!(outgoing.len(), 2);

    let targets: Vec<u64> = outgoing.iter().map(|e| e.target()).collect();
    assert!(targets.contains(&2));
    assert!(targets.contains(&3));
}

#[test]
fn test_get_incoming() {
    let store = build_test_graph();
    let incoming = store.get_incoming(3);
    assert_eq!(incoming.len(), 2);

    let sources: Vec<u64> = incoming.iter().map(|e| e.source()).collect();
    assert!(sources.contains(&1));
    assert!(sources.contains(&2));
}

#[test]
fn test_get_outgoing_by_label() {
    let store = build_test_graph();
    let works_at = store.get_outgoing_by_label(1, "WORKS_AT");
    assert_eq!(works_at.len(), 1);
    assert_eq!(works_at[0].target(), 3);

    let knows = store.get_outgoing_by_label(1, "KNOWS");
    assert_eq!(knows.len(), 1);
    assert_eq!(knows[0].target(), 2);
}

#[test]
fn test_get_edges_by_label() {
    let store = build_test_graph();
    let works_at = store.get_edges_by_label("WORKS_AT");
    assert_eq!(works_at.len(), 2);
}

#[test]
fn test_remove_edge() {
    let mut store = build_test_graph();
    let removed = store.remove_edge(100);
    assert!(removed.is_some());
    assert_eq!(store.edge_count(), 2);
    assert!(!store.has_edge(100));

    // Outgoing from node 1 should now only have WORKS_AT
    let outgoing = store.get_outgoing(1);
    assert_eq!(outgoing.len(), 1);
    assert_eq!(outgoing[0].label(), "WORKS_AT");
}

#[test]
fn test_out_degree_in_degree() {
    let store = build_test_graph();
    assert_eq!(store.out_degree(1), 2);
    assert_eq!(store.out_degree(2), 1);
    assert_eq!(store.out_degree(3), 0);
    assert_eq!(store.in_degree(1), 0);
    assert_eq!(store.in_degree(2), 1);
    assert_eq!(store.in_degree(3), 2);
}

#[test]
fn test_get_nodes_by_label() {
    let store = build_test_graph();
    let persons = store.get_nodes_by_label("Person");
    assert_eq!(persons.len(), 2);

    let companies = store.get_nodes_by_label("Company");
    assert_eq!(companies.len(), 1);
}

#[test]
fn test_clear() {
    let mut store = build_test_graph();
    store.clear();
    assert_eq!(store.node_count(), 0);
    assert_eq!(store.edge_count(), 0);
}

#[test]
fn test_with_capacity() {
    let store = InMemoryEdgeStore::with_capacity(1000, 100);
    assert_eq!(store.node_count(), 0);
    assert_eq!(store.edge_count(), 0);
}

#[test]
fn test_empty_store() {
    let store = InMemoryEdgeStore::new();
    assert_eq!(store.get_outgoing(999).len(), 0);
    assert_eq!(store.get_incoming(999).len(), 0);
    assert!(store.get_node(999).is_none());
    assert!(store.get_edge(999).is_none());
    assert_eq!(store.out_degree(999), 0);
    assert_eq!(store.in_degree(999), 0);
}
