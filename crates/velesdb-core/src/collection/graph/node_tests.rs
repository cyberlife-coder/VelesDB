//! Tests for the GraphNode type.
//!
//! TDD: Tests written BEFORE implementation (RED phase).

use super::*;
use serde_json::json;
use std::collections::HashMap;

// =============================================================================
// AC-1: GraphNode creation and properties
// =============================================================================

#[test]
fn test_create_graph_node_with_label() {
    let node = GraphNode::new(1, "Person");

    assert_eq!(node.id(), 1);
    assert_eq!(node.label(), "Person");
    assert!(node.properties().is_empty());
    assert!(node.vector().is_none());
    assert_eq!(node.property("name"), None);
}

#[test]
fn test_graph_node_with_properties() {
    let mut props = HashMap::new();
    props.insert("name".to_string(), json!("Alice"));
    props.insert("age".to_string(), json!(30));

    let node = GraphNode::new(1, "Person").with_properties(props);

    assert_eq!(node.properties().len(), 2);
    assert_eq!(node.property("name"), Some(&json!("Alice")));
    assert_eq!(node.property("age"), Some(&json!(30)));
    assert_eq!(node.property("unknown"), None);
}

#[test]
fn test_graph_node_builder_pattern() {
    let mut props = HashMap::new();
    props.insert("title".to_string(), json!("Research Paper"));

    let node = GraphNode::new(42, "Article")
        .with_properties(props)
        .with_vector(vec![0.5; 128]);

    assert_eq!(node.id(), 42);
    assert_eq!(node.label(), "Article");
    assert_eq!(node.properties().len(), 1);
    assert!(node.vector().is_some());
    assert_eq!(node.vector().unwrap().len(), 128);
}

// =============================================================================
// Serialization tests
// =============================================================================

#[test]
fn test_graph_node_serialization_roundtrip() {
    let mut props = HashMap::new();
    props.insert("name".to_string(), json!("Test"));

    let node = GraphNode::new(123, "TestType")
        .with_properties(props)
        .with_vector(vec![1.0, 2.0, 3.0]);

    let json = serde_json::to_string(&node).expect("serialization failed");
    let deserialized: GraphNode = serde_json::from_str(&json).expect("deserialization failed");

    assert_eq!(node.id(), deserialized.id());
    assert_eq!(node.label(), deserialized.label());
    assert_eq!(node.properties().len(), deserialized.properties().len());
    assert_eq!(node.vector(), deserialized.vector());
}

// =============================================================================
// Edge cases
// =============================================================================

#[test]
fn test_graph_node_empty_label() {
    let node = GraphNode::new(1, "");
    assert_eq!(node.label(), "");
}

#[test]
fn test_graph_node_empty_properties() {
    let mut props = HashMap::new();
    props.insert("name".to_string(), json!("Alice"));
    let node = GraphNode::new(1, "Type")
        .with_properties(props)
        .with_properties(HashMap::new());
    assert!(node.properties().is_empty());
}

#[test]
fn test_graph_node_empty_vector() {
    let node = GraphNode::new(1, "Type").with_vector(vec![]);
    assert!(node.vector().is_some());
    assert!(node.vector().unwrap().is_empty());
}
