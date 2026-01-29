//! Tests for WASM Graph module (EPIC-061/US-006 refactoring).
//!
//! Extracted from graph.rs to improve modularity.

use super::*;
use wasm_bindgen_test::*;

#[wasm_bindgen_test]
fn test_graph_node_creation() {
    let node = GraphNode::new(1, "Person");
    assert_eq!(node.id(), 1);
    assert_eq!(node.label(), "Person");
    assert!(!node.has_vector());
}

#[wasm_bindgen_test]
fn test_graph_node_properties() {
    let mut node = GraphNode::new(1, "Person");
    node.set_string_property("name", "John");
    node.set_number_property("age", 30.0);
    node.set_bool_property("active", true);

    assert!(node.properties.contains_key("name"));
    assert!(node.properties.contains_key("age"));
    assert!(node.properties.contains_key("active"));
}

#[wasm_bindgen_test]
fn test_graph_node_vector() {
    let mut node = GraphNode::new(1, "Document");
    assert!(!node.has_vector());

    node.set_vector(vec![0.1, 0.2, 0.3]);
    assert!(node.has_vector());
}

#[wasm_bindgen_test]
fn test_graph_edge_creation() {
    let edge = GraphEdge::new(100, 1, 2, "KNOWS").unwrap();
    assert_eq!(edge.id(), 100);
    assert_eq!(edge.source(), 1);
    assert_eq!(edge.target(), 2);
    assert_eq!(edge.label(), "KNOWS");
}

#[wasm_bindgen_test]
fn test_graph_edge_empty_label_error() {
    let result = GraphEdge::new(100, 1, 2, "  ");
    assert!(result.is_err());
}

#[wasm_bindgen_test]
fn test_graph_store_add_nodes() {
    let mut store = GraphStore::new();
    store.add_node(GraphNode::new(1, "Person"));
    store.add_node(GraphNode::new(2, "Person"));

    assert_eq!(store.node_count(), 2);
}

#[wasm_bindgen_test]
fn test_graph_store_add_edges() {
    let mut store = GraphStore::new();
    store.add_node(GraphNode::new(1, "Person"));
    store.add_node(GraphNode::new(2, "Person"));
    store
        .add_edge(GraphEdge::new(100, 1, 2, "KNOWS").unwrap())
        .unwrap();

    assert_eq!(store.edge_count(), 1);
}

#[wasm_bindgen_test]
fn test_graph_store_get_outgoing() {
    let mut store = GraphStore::new();
    store.add_node(GraphNode::new(1, "Person"));
    store.add_node(GraphNode::new(2, "Person"));
    store.add_node(GraphNode::new(3, "Person"));
    store
        .add_edge(GraphEdge::new(100, 1, 2, "KNOWS").unwrap())
        .unwrap();
    store
        .add_edge(GraphEdge::new(101, 1, 3, "KNOWS").unwrap())
        .unwrap();

    let outgoing = store.get_outgoing(1);
    assert_eq!(outgoing.len(), 2);
}

#[wasm_bindgen_test]
fn test_graph_store_get_neighbors() {
    let mut store = GraphStore::new();
    store.add_node(GraphNode::new(1, "Person"));
    store.add_node(GraphNode::new(2, "Person"));
    store.add_node(GraphNode::new(3, "Person"));
    store
        .add_edge(GraphEdge::new(100, 1, 2, "KNOWS").unwrap())
        .unwrap();
    store
        .add_edge(GraphEdge::new(101, 1, 3, "KNOWS").unwrap())
        .unwrap();

    let neighbors = store.get_neighbors(1);
    assert_eq!(neighbors.len(), 2);
    assert!(neighbors.contains(&2));
    assert!(neighbors.contains(&3));
}

#[wasm_bindgen_test]
fn test_graph_store_remove_node() {
    let mut store = GraphStore::new();
    store.add_node(GraphNode::new(1, "Person"));
    store.add_node(GraphNode::new(2, "Person"));
    store
        .add_edge(GraphEdge::new(100, 1, 2, "KNOWS").unwrap())
        .unwrap();

    store.remove_node(1);

    assert_eq!(store.node_count(), 1);
    assert_eq!(store.edge_count(), 0);
}

#[wasm_bindgen_test]
fn test_graph_store_duplicate_edge_error() {
    let mut store = GraphStore::new();
    store.add_node(GraphNode::new(1, "Person"));
    store.add_node(GraphNode::new(2, "Person"));
    store
        .add_edge(GraphEdge::new(100, 1, 2, "KNOWS").unwrap())
        .unwrap();

    let result = store.add_edge(GraphEdge::new(100, 1, 2, "KNOWS").unwrap());
    assert!(result.is_err());
}

#[wasm_bindgen_test]
fn test_graph_store_get_node() {
    let mut store = GraphStore::new();
    store.add_node(GraphNode::new(1, "Person"));

    let node = store.get_node(1);
    assert!(node.is_some());
    assert_eq!(node.unwrap().id(), 1);

    let missing = store.get_node(999);
    assert!(missing.is_none());
}

#[wasm_bindgen_test]
fn test_graph_store_get_edge() {
    let mut store = GraphStore::new();
    store.add_node(GraphNode::new(1, "Person"));
    store.add_node(GraphNode::new(2, "Person"));
    store
        .add_edge(GraphEdge::new(100, 1, 2, "KNOWS").unwrap())
        .unwrap();

    let edge = store.get_edge(100);
    assert!(edge.is_some());
    assert_eq!(edge.unwrap().label(), "KNOWS");

    let missing = store.get_edge(999);
    assert!(missing.is_none());
}

#[wasm_bindgen_test]
fn test_graph_store_get_incoming() {
    let mut store = GraphStore::new();
    store.add_node(GraphNode::new(1, "Person"));
    store.add_node(GraphNode::new(2, "Person"));
    store.add_node(GraphNode::new(3, "Person"));
    store
        .add_edge(GraphEdge::new(100, 1, 3, "KNOWS").unwrap())
        .unwrap();
    store
        .add_edge(GraphEdge::new(101, 2, 3, "KNOWS").unwrap())
        .unwrap();

    let incoming = store.get_incoming(3);
    assert_eq!(incoming.len(), 2);
}

#[wasm_bindgen_test]
fn test_graph_store_get_outgoing_by_label() {
    let mut store = GraphStore::new();
    store.add_node(GraphNode::new(1, "Person"));
    store.add_node(GraphNode::new(2, "Person"));
    store.add_node(GraphNode::new(3, "Company"));
    store
        .add_edge(GraphEdge::new(100, 1, 2, "KNOWS").unwrap())
        .unwrap();
    store
        .add_edge(GraphEdge::new(101, 1, 3, "WORKS_AT").unwrap())
        .unwrap();

    let knows = store.get_outgoing_by_label(1, "KNOWS");
    assert_eq!(knows.len(), 1);
    assert_eq!(knows[0].target(), 2);

    let works = store.get_outgoing_by_label(1, "WORKS_AT");
    assert_eq!(works.len(), 1);
    assert_eq!(works[0].target(), 3);
}

#[wasm_bindgen_test]
fn test_graph_store_remove_edge() {
    let mut store = GraphStore::new();
    store.add_node(GraphNode::new(1, "Person"));
    store.add_node(GraphNode::new(2, "Person"));
    store
        .add_edge(GraphEdge::new(100, 1, 2, "KNOWS").unwrap())
        .unwrap();

    assert_eq!(store.edge_count(), 1);
    store.remove_edge(100);
    assert_eq!(store.edge_count(), 0);
    assert!(store.get_outgoing(1).is_empty());
}

#[wasm_bindgen_test]
fn test_graph_store_clear() {
    let mut store = GraphStore::new();
    store.add_node(GraphNode::new(1, "Person"));
    store.add_node(GraphNode::new(2, "Person"));
    store
        .add_edge(GraphEdge::new(100, 1, 2, "KNOWS").unwrap())
        .unwrap();

    assert_eq!(store.node_count(), 2);
    assert_eq!(store.edge_count(), 1);

    store.clear();

    assert_eq!(store.node_count(), 0);
    assert_eq!(store.edge_count(), 0);
}

#[wasm_bindgen_test]
fn test_dfs_traverse() {
    let mut store = GraphStore::new();
    store.add_node(GraphNode::new(1, "A"));
    store.add_node(GraphNode::new(2, "B"));
    store.add_node(GraphNode::new(3, "C"));
    store.add_node(GraphNode::new(4, "D"));
    store
        .add_edge(GraphEdge::new(100, 1, 2, "NEXT").unwrap())
        .unwrap();
    store
        .add_edge(GraphEdge::new(101, 2, 3, "NEXT").unwrap())
        .unwrap();
    store
        .add_edge(GraphEdge::new(102, 1, 4, "NEXT").unwrap())
        .unwrap();

    let result = store.dfs_traverse(1, 3, 10);
    assert!(result.is_ok());
}

#[wasm_bindgen_test]
fn test_get_nodes_by_label() {
    let mut store = GraphStore::new();
    store.add_node(GraphNode::new(1, "Person"));
    store.add_node(GraphNode::new(2, "Person"));
    store.add_node(GraphNode::new(3, "Document"));

    let persons = store.get_nodes_by_label("Person");
    assert_eq!(persons.len(), 2);

    let docs = store.get_nodes_by_label("Document");
    assert_eq!(docs.len(), 1);
}

#[wasm_bindgen_test]
fn test_get_edges_by_label() {
    let mut store = GraphStore::new();
    store.add_node(GraphNode::new(1, "Person"));
    store.add_node(GraphNode::new(2, "Person"));
    store.add_node(GraphNode::new(3, "Person"));
    store
        .add_edge(GraphEdge::new(100, 1, 2, "KNOWS").unwrap())
        .unwrap();
    store
        .add_edge(GraphEdge::new(101, 2, 3, "WORKS_WITH").unwrap())
        .unwrap();

    let knows = store.get_edges_by_label("KNOWS");
    assert_eq!(knows.len(), 1);

    let works = store.get_edges_by_label("WORKS_WITH");
    assert_eq!(works.len(), 1);
}

#[wasm_bindgen_test]
fn test_has_node_and_edge() {
    let mut store = GraphStore::new();
    store.add_node(GraphNode::new(1, "Person"));
    store
        .add_edge(GraphEdge::new(100, 1, 1, "SELF").unwrap())
        .unwrap();

    assert!(store.has_node(1));
    assert!(!store.has_node(999));
    assert!(store.has_edge(100));
    assert!(!store.has_edge(999));
}

#[wasm_bindgen_test]
fn test_degree() {
    let mut store = GraphStore::new();
    store.add_node(GraphNode::new(1, "A"));
    store.add_node(GraphNode::new(2, "B"));
    store.add_node(GraphNode::new(3, "C"));
    store
        .add_edge(GraphEdge::new(100, 1, 2, "E").unwrap())
        .unwrap();
    store
        .add_edge(GraphEdge::new(101, 1, 3, "E").unwrap())
        .unwrap();
    store
        .add_edge(GraphEdge::new(102, 2, 1, "E").unwrap())
        .unwrap();

    assert_eq!(store.out_degree(1), 2);
    assert_eq!(store.in_degree(1), 1);
    assert_eq!(store.out_degree(2), 1);
    assert_eq!(store.in_degree(2), 1);
}
