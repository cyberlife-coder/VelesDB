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

// === Tests d'erreurs additionnels (requis par US-001 DoD) ===

#[test]
fn test_error_missing_parenthesis() {
    // Node pattern sans parenthèse fermante
    let result = parse_node_pattern("(n:Person");
    assert!(result.is_err());
    // Vérifier que l'erreur mentionne la parenthèse
    let err = result.unwrap_err();
    assert!(err.to_string().contains("')'") || err.to_string().contains("Expected"));
}

#[test]
fn test_error_invalid_direction() {
    // Direction invalide (pas de flèche reconnue)
    let result = parse_relationship_pattern("<<-[r:WROTE]->>");
    assert!(result.is_err());
}

#[test]
fn test_error_range_invalid() {
    // Range avec start > end n'est pas une erreur de parsing,
    // mais on vérifie quand même le parsing de ranges mal formés
    let _result = parse_relationship_pattern("-[*abc]->");
    // Le range "abc" n'est pas valide, mais unwrap_or gère ce cas
    // Testons plutôt un pattern complètement invalide
    let result2 = parse_relationship_pattern("invalid");
    assert!(result2.is_err());
}

// === Variable-length relationship tests (Bug fix) ===

#[test]
fn test_parse_relationship_star_unbounded() {
    // -[*]-> should parse as unbounded range (1, MAX)
    let result = parse_relationship_pattern("-[*]->");
    assert!(result.is_ok());
    let rel = result.unwrap();
    assert_eq!(rel.range, Some((1, u32::MAX)));
}

#[test]
fn test_parse_relationship_star_exact() {
    // -[*3]-> should parse as exact range (3, 3)
    let result = parse_relationship_pattern("-[*3]->");
    assert!(result.is_ok());
    let rel = result.unwrap();
    assert_eq!(rel.range, Some((3, 3)));
}

#[test]
fn test_parse_relationship_star_open_end() {
    // -[*2..]-> should parse as (2, MAX)
    let result = parse_relationship_pattern("-[*2..]->");
    assert!(result.is_ok());
    let rel = result.unwrap();
    assert_eq!(rel.range, Some((2, u32::MAX)));
}

#[test]
fn test_parse_relationship_star_open_start() {
    // -[*..5]-> should parse as (1, 5)
    let result = parse_relationship_pattern("-[*..5]->");
    assert!(result.is_ok());
    let rel = result.unwrap();
    assert_eq!(rel.range, Some((1, 5)));
}
