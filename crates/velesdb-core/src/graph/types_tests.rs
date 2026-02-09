//! Tests for graph types (GraphNode, GraphEdge).

use super::types::{GraphEdge, GraphNode};
use serde_json::json;
use std::collections::HashMap;

#[test]
fn test_graph_node_new() {
    let node = GraphNode::new(1, "Person");
    assert_eq!(node.id(), 1);
    assert_eq!(node.label(), "Person");
    assert!(node.properties().is_empty());
    assert!(node.vector().is_none());
}

#[test]
fn test_graph_node_with_properties() {
    let mut props = HashMap::new();
    props.insert("name".to_string(), json!("Alice"));
    props.insert("age".to_string(), json!(30));

    let node = GraphNode::new(1, "Person").with_properties(props);
    assert_eq!(node.property("name"), Some(&json!("Alice")));
    assert_eq!(node.property("age"), Some(&json!(30)));
    assert_eq!(node.property("missing"), None);
}

#[test]
fn test_graph_node_with_vector() {
    let node = GraphNode::new(1, "Entity").with_vector(vec![0.1, 0.2, 0.3]);
    let vec = node.vector().unwrap();
    assert_eq!(vec.len(), 3);
}

#[test]
fn test_graph_node_set_property() {
    let mut node = GraphNode::new(1, "Person");
    node.set_property("name", json!("Bob"));
    assert_eq!(node.property("name"), Some(&json!("Bob")));
}

#[test]
fn test_graph_node_serialize_deserialize() {
    let node = GraphNode::new(1, "Person")
        .with_properties({
            let mut p = HashMap::new();
            p.insert("name".to_string(), json!("Alice"));
            p
        })
        .with_vector(vec![1.0, 2.0]);

    let json_str = serde_json::to_string(&node).unwrap();
    let restored: GraphNode = serde_json::from_str(&json_str).unwrap();
    assert_eq!(node, restored);
}

#[test]
fn test_graph_edge_new() {
    let edge = GraphEdge::new(1, 100, 200, "KNOWS").unwrap();
    assert_eq!(edge.id(), 1);
    assert_eq!(edge.source(), 100);
    assert_eq!(edge.target(), 200);
    assert_eq!(edge.label(), "KNOWS");
    assert!(edge.properties().is_empty());
}

#[test]
fn test_graph_edge_with_properties() {
    let mut props = HashMap::new();
    props.insert("since".to_string(), json!("2020-01-01"));

    let edge = GraphEdge::new(1, 100, 200, "KNOWS")
        .unwrap()
        .with_properties(props);
    assert_eq!(edge.property("since"), Some(&json!("2020-01-01")));
}

#[test]
fn test_graph_edge_empty_label_rejected() {
    let result = GraphEdge::new(1, 100, 200, "");
    assert!(result.is_err());
}

#[test]
fn test_graph_edge_whitespace_label_rejected() {
    let result = GraphEdge::new(1, 100, 200, "   ");
    assert!(result.is_err());
}

#[test]
fn test_graph_edge_label_trimmed() {
    let edge = GraphEdge::new(1, 100, 200, "  KNOWS  ").unwrap();
    assert_eq!(edge.label(), "KNOWS");
}

#[test]
fn test_graph_edge_serialize_deserialize() {
    let edge = GraphEdge::new(1, 100, 200, "WORKS_AT").unwrap();
    let json_str = serde_json::to_string(&edge).unwrap();
    let restored: GraphEdge = serde_json::from_str(&json_str).unwrap();
    assert_eq!(edge, restored);
}
