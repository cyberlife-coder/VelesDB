//! Tests for MATCH clause parser.

use super::match_clause::{parse_match_clause, parse_node_pattern, parse_relationship_pattern};
use crate::velesql::ast::Direction;

#[test]
fn test_parse_simple_node() {
    let result = parse_node_pattern("(n)");
    assert!(result.is_ok());
    let node = result.unwrap();
    assert_eq!(node.alias, Some("n".to_string()));
}

#[test]
fn test_parse_node_with_label() {
    let result = parse_node_pattern("(n:Person)");
    assert!(result.is_ok());
    let node = result.unwrap();
    assert_eq!(node.labels, vec!["Person".to_string()]);
}

#[test]
fn test_parse_node_multi_labels() {
    let result = parse_node_pattern("(n:Person:Author)");
    assert!(result.is_ok());
    let node = result.unwrap();
    assert_eq!(
        node.labels,
        vec!["Person".to_string(), "Author".to_string()]
    );
}

#[test]
fn test_parse_node_with_properties() {
    let result = parse_node_pattern("(n:Person {name: 'Alice', age: 30})");
    assert!(result.is_ok());
    let node = result.unwrap();
    assert!(node.properties.contains_key("name"));
}

#[test]
fn test_parse_anonymous_node() {
    let result = parse_node_pattern("()");
    assert!(result.is_ok());
    assert!(result.unwrap().alias.is_none());
}

#[test]
fn test_parse_relationship_outgoing() {
    let result = parse_relationship_pattern("-[r:WROTE]->");
    assert!(result.is_ok());
    let rel = result.unwrap();
    assert_eq!(rel.direction, Direction::Outgoing);
    assert_eq!(rel.types, vec!["WROTE".to_string()]);
}

#[test]
fn test_parse_relationship_incoming() {
    let result = parse_relationship_pattern("<-[r:WROTE]-");
    assert!(result.is_ok());
    assert_eq!(result.unwrap().direction, Direction::Incoming);
}

#[test]
fn test_parse_relationship_undirected() {
    let result = parse_relationship_pattern("-[r:KNOWS]-");
    assert!(result.is_ok());
    assert_eq!(result.unwrap().direction, Direction::Both);
}

#[test]
fn test_parse_relationship_multi_types() {
    let result = parse_relationship_pattern("-[:WROTE|CREATED]->");
    assert!(result.is_ok());
    assert_eq!(
        result.unwrap().types,
        vec!["WROTE".to_string(), "CREATED".to_string()]
    );
}

#[test]
fn test_parse_relationship_with_range() {
    let result = parse_relationship_pattern("-[*1..3]->");
    assert!(result.is_ok());
    assert_eq!(result.unwrap().range, Some((1, 3)));
}

#[test]
fn test_parse_match_simple() {
    let result = parse_match_clause("MATCH (p:Person)-[:WROTE]->(a:Article) RETURN a.title");
    assert!(result.is_ok());
    let mc = result.unwrap();
    assert_eq!(mc.patterns[0].nodes.len(), 2);
    assert_eq!(mc.patterns[0].relationships.len(), 1);
}

#[test]
fn test_parse_match_with_where() {
    let result = parse_match_clause("MATCH (p:Person)-[:WROTE]->(a) WHERE p.age > 18 RETURN a");
    assert!(result.is_ok());
    assert!(result.unwrap().where_clause.is_some());
}

#[test]
fn test_parse_match_named_path() {
    let result = parse_match_clause("MATCH path = (a)-[*1..5]->(b) RETURN path");
    assert!(result.is_ok());
    assert_eq!(result.unwrap().patterns[0].name, Some("path".to_string()));
}

#[test]
fn test_error_missing_return() {
    assert!(parse_match_clause("MATCH (n:Person)").is_err());
}

#[test]
fn test_error_empty_pattern() {
    assert!(parse_match_clause("MATCH RETURN n").is_err());
}
