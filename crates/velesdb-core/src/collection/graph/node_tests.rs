//! Tests for GraphNode and Element types.
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
fn test_graph_node_with_optional_vector() {
    let vector = vec![0.1, 0.2, 0.3, 0.4];
    let node = GraphNode::new(1, "Document").with_vector(vector.clone());

    assert!(node.vector().is_some());
    assert_eq!(node.vector().unwrap(), &vector);
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
// AC-2: Element enum - distinction Point/Node
// =============================================================================

#[test]
fn test_element_from_point() {
    let point = crate::Point::new(1, vec![0.1, 0.2, 0.3], Some(json!({"text": "hello"})));
    let element = Element::Point(point);

    assert!(element.is_point());
    assert!(!element.is_node());
    assert_eq!(element.id(), 1);
}

#[test]
fn test_element_from_node() {
    let node = GraphNode::new(2, "Person");
    let element = Element::Node(node);

    assert!(!element.is_point());
    assert!(element.is_node());
    assert_eq!(element.id(), 2);
}

#[test]
fn test_element_as_point() {
    let point = crate::Point::new(1, vec![0.1, 0.2], None);
    let element = Element::Point(point);

    let retrieved = element.as_point();
    assert!(retrieved.is_some());
    assert_eq!(retrieved.unwrap().id, 1);

    assert!(element.as_node().is_none());
}

#[test]
fn test_element_as_node() {
    let node = GraphNode::new(2, "Company");
    let element = Element::Node(node);

    let retrieved = element.as_node();
    assert!(retrieved.is_some());
    assert_eq!(retrieved.unwrap().label(), "Company");

    assert!(element.as_point().is_none());
}

#[test]
fn test_element_has_vector() {
    // Point always has vector
    let point = crate::Point::new(1, vec![0.1, 0.2], None);
    let elem_point = Element::Point(point);
    assert!(elem_point.has_vector());

    // Node without vector
    let node_no_vec = GraphNode::new(2, "Person");
    let elem_no_vec = Element::Node(node_no_vec);
    assert!(!elem_no_vec.has_vector());

    // Node with vector
    let node_with_vec = GraphNode::new(3, "Document").with_vector(vec![0.5; 64]);
    let elem_with_vec = Element::Node(node_with_vec);
    assert!(elem_with_vec.has_vector());
}

#[test]
fn test_element_get_vector() {
    let point = crate::Point::new(1, vec![0.1, 0.2, 0.3], None);
    let elem = Element::Point(point);
    let vec = elem.vector();
    assert!(vec.is_some());
    assert_eq!(vec.unwrap().len(), 3);

    let node = GraphNode::new(2, "Entity");
    let elem_node = Element::Node(node);
    assert!(elem_node.vector().is_none());
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

#[test]
fn test_element_serialization_roundtrip() {
    let node = GraphNode::new(1, "Person");
    let element = Element::Node(node);

    let json = serde_json::to_string(&element).expect("serialization failed");
    let deserialized: Element = serde_json::from_str(&json).expect("deserialization failed");

    assert!(deserialized.is_node());
    assert_eq!(element.id(), deserialized.id());
}

// =============================================================================
// ID uniqueness and validation
// =============================================================================

#[test]
fn test_element_id_uniqueness_concept() {
    // Both Point and Node use the same ID space
    let point = crate::Point::new(100, vec![0.1], None);
    let node = GraphNode::new(100, "Entity");

    // They have the same ID - in a collection, this would be a conflict
    assert_eq!(Element::Point(point).id(), Element::Node(node).id());
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
    let node = GraphNode::new(1, "Type").with_properties(HashMap::new());
    assert!(node.properties().is_empty());
}

#[test]
fn test_graph_node_empty_vector() {
    let node = GraphNode::new(1, "Type").with_vector(vec![]);
    assert!(node.vector().is_some());
    assert!(node.vector().unwrap().is_empty());
}
