//! Tests for graph_pattern module.

use super::graph_pattern::{Direction, NodePattern, RelationshipPattern};

#[test]
fn test_node_pattern_new() {
    let node = NodePattern::new();
    assert!(node.alias.is_none());
    assert!(node.labels.is_empty());
    assert!(node.properties.is_empty());
}

#[test]
fn test_node_pattern_with_alias() {
    let node = NodePattern::new().with_alias("n");
    assert_eq!(node.alias, Some("n".to_string()));
}

#[test]
fn test_node_pattern_with_label() {
    let node = NodePattern::new().with_label("Person");
    assert_eq!(node.labels, vec!["Person".to_string()]);
}

#[test]
fn test_node_pattern_builder_chain() {
    let node = NodePattern::new()
        .with_alias("p")
        .with_label("Person")
        .with_label("Employee");

    assert_eq!(node.alias, Some("p".to_string()));
    assert_eq!(
        node.labels,
        vec!["Person".to_string(), "Employee".to_string()]
    );
}

#[test]
fn test_node_pattern_default() {
    let node = NodePattern::default();
    assert!(node.alias.is_none());
    assert!(node.labels.is_empty());
}

#[test]
fn test_relationship_pattern_new() {
    let rel = RelationshipPattern::new(Direction::Outgoing);
    assert!(rel.alias.is_none());
    assert!(rel.types.is_empty());
    assert_eq!(rel.direction, Direction::Outgoing);
    assert!(rel.range.is_none());
    assert!(rel.properties.is_empty());
}
