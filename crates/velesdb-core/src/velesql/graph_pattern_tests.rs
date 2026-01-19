//! Tests for graph_pattern module.

use super::graph_pattern::{
    Direction, GraphPattern, MatchClause, NodePattern, OrderByItem, RelationshipPattern,
    ReturnClause, ReturnItem,
};
use std::collections::HashMap;

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

#[test]
fn test_direction_variants() {
    assert_eq!(Direction::Outgoing, Direction::Outgoing);
    assert_eq!(Direction::Incoming, Direction::Incoming);
    assert_eq!(Direction::Both, Direction::Both);
    assert_ne!(Direction::Outgoing, Direction::Incoming);
}

#[test]
fn test_graph_pattern_structure() {
    let pattern = GraphPattern {
        name: Some("path".to_string()),
        nodes: vec![NodePattern::new().with_alias("a")],
        relationships: vec![RelationshipPattern::new(Direction::Outgoing)],
    };

    assert_eq!(pattern.name, Some("path".to_string()));
    assert_eq!(pattern.nodes.len(), 1);
    assert_eq!(pattern.relationships.len(), 1);
}

#[test]
fn test_return_clause_structure() {
    let return_clause = ReturnClause {
        items: vec![ReturnItem {
            expression: "n.name".to_string(),
            alias: Some("name".to_string()),
        }],
        order_by: Some(vec![OrderByItem {
            expression: "n.age".to_string(),
            descending: true,
        }]),
        limit: Some(10),
    };

    assert_eq!(return_clause.items.len(), 1);
    assert_eq!(return_clause.items[0].expression, "n.name");
    assert_eq!(return_clause.items[0].alias, Some("name".to_string()));
    assert!(return_clause.order_by.is_some());
    assert_eq!(return_clause.limit, Some(10));
}

#[test]
fn test_match_clause_structure() {
    let match_clause = MatchClause {
        patterns: vec![GraphPattern {
            name: None,
            nodes: vec![NodePattern::new().with_alias("n").with_label("Person")],
            relationships: vec![],
        }],
        where_clause: None,
        return_clause: ReturnClause {
            items: vec![ReturnItem {
                expression: "n".to_string(),
                alias: None,
            }],
            order_by: None,
            limit: None,
        },
    };

    assert_eq!(match_clause.patterns.len(), 1);
    assert!(match_clause.where_clause.is_none());
    assert_eq!(match_clause.return_clause.items.len(), 1);
}

#[test]
fn test_return_item_without_alias() {
    let item = ReturnItem {
        expression: "count(*)".to_string(),
        alias: None,
    };

    assert_eq!(item.expression, "count(*)");
    assert!(item.alias.is_none());
}

#[test]
fn test_order_by_ascending() {
    let item = OrderByItem {
        expression: "n.created_at".to_string(),
        descending: false,
    };

    assert!(!item.descending);
}

#[test]
fn test_relationship_pattern_with_range() {
    let rel = RelationshipPattern {
        alias: Some("r".to_string()),
        types: vec!["KNOWS".to_string()],
        direction: Direction::Both,
        range: Some((1, 5)),
        properties: HashMap::new(),
    };

    assert_eq!(rel.range, Some((1, 5)));
    assert_eq!(rel.direction, Direction::Both);
}
