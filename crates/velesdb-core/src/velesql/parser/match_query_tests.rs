//! Tests for MATCH query parsing (EPIC-045 US-001).

use crate::velesql::graph_pattern::Direction;
use crate::velesql::Parser;

#[test]
fn test_parse_match_simple() {
    let query = Parser::parse("MATCH (n:Person) RETURN n").unwrap();
    assert!(query.is_match_query());
    let mc = query.match_clause.unwrap();
    assert_eq!(mc.patterns.len(), 1);
    assert_eq!(mc.patterns[0].nodes.len(), 1);
    assert_eq!(mc.patterns[0].nodes[0].labels, vec!["Person".to_string()]);
    assert_eq!(mc.patterns[0].nodes[0].alias, Some("n".to_string()));
}

#[test]
fn test_parse_match_with_relationship() {
    let query = Parser::parse("MATCH (a:Author)-[:WROTE]->(b:Book) RETURN a, b").unwrap();
    assert!(query.is_match_query());
    let mc = query.match_clause.unwrap();
    assert_eq!(mc.patterns[0].nodes.len(), 2);
    assert_eq!(mc.patterns[0].relationships.len(), 1);
    assert_eq!(
        mc.patterns[0].relationships[0].types,
        vec!["WROTE".to_string()]
    );
    assert_eq!(
        mc.patterns[0].relationships[0].direction,
        Direction::Outgoing
    );
}

#[test]
fn test_parse_match_incoming_relationship() {
    let query = Parser::parse("MATCH (a)<-[:FOLLOWS]-(b) RETURN a").unwrap();
    assert!(query.is_match_query());
    let mc = query.match_clause.unwrap();
    assert_eq!(
        mc.patterns[0].relationships[0].direction,
        Direction::Incoming
    );
}

#[test]
fn test_parse_match_with_where() {
    // TODO: EPIC-045 US-004 - Support property access (n.age) in WHERE
    // For now, test with simple column comparison
    let query = Parser::parse("MATCH (n:Person) WHERE age > 18 RETURN n").unwrap();
    assert!(query.is_match_query());
    let mc = query.match_clause.unwrap();
    assert!(mc.where_clause.is_some());
}

#[test]
fn test_parse_match_with_limit() {
    let query = Parser::parse("MATCH (n) RETURN n LIMIT 10").unwrap();
    assert!(query.is_match_query());
    let mc = query.match_clause.unwrap();
    assert_eq!(mc.return_clause.limit, Some(10));
}

#[test]
fn test_parse_match_with_properties() {
    let query = Parser::parse("MATCH (n:Person {name: 'Alice', age: 30}) RETURN n").unwrap();
    assert!(query.is_match_query());
    let mc = query.match_clause.unwrap();
    let node = &mc.patterns[0].nodes[0];
    assert_eq!(node.properties.len(), 2);
    assert!(node.properties.contains_key("name"));
    assert!(node.properties.contains_key("age"));
}

#[test]
fn test_parse_match_variable_length() {
    let query = Parser::parse("MATCH (a)-[*1..3]->(b) RETURN a, b").unwrap();
    assert!(query.is_match_query());
    let mc = query.match_clause.unwrap();
    let rel = &mc.patterns[0].relationships[0];
    assert_eq!(rel.range, Some((1, 3)));
}

#[test]
fn test_parse_match_multiple_labels() {
    let query = Parser::parse("MATCH (n:Person:Employee) RETURN n").unwrap();
    assert!(query.is_match_query());
    let mc = query.match_clause.unwrap();
    assert_eq!(
        mc.patterns[0].nodes[0].labels,
        vec!["Person".to_string(), "Employee".to_string()]
    );
}

#[test]
fn test_parse_match_return_property() {
    let query = Parser::parse("MATCH (n:Person) RETURN n.name").unwrap();
    assert!(query.is_match_query());
    let mc = query.match_clause.unwrap();
    assert_eq!(mc.return_clause.items.len(), 1);
    assert_eq!(mc.return_clause.items[0].expression, "n.name");
}

#[test]
fn test_parse_match_return_with_alias() {
    let query = Parser::parse("MATCH (n:Person) RETURN n.name AS personName").unwrap();
    assert!(query.is_match_query());
    let mc = query.match_clause.unwrap();
    assert_eq!(
        mc.return_clause.items[0].alias,
        Some("personName".to_string())
    );
}

#[test]
fn test_parse_match_undirected() {
    let query = Parser::parse("MATCH (a)-[:KNOWS]-(b) RETURN a, b").unwrap();
    assert!(query.is_match_query());
    let mc = query.match_clause.unwrap();
    assert_eq!(mc.patterns[0].relationships[0].direction, Direction::Both);
}

#[test]
fn test_parse_match_relationship_alias() {
    let query = Parser::parse("MATCH (a)-[r:WROTE]->(b) RETURN r").unwrap();
    assert!(query.is_match_query());
    let mc = query.match_clause.unwrap();
    assert_eq!(mc.patterns[0].relationships[0].alias, Some("r".to_string()));
}

#[test]
fn test_select_query_not_match() {
    let query = Parser::parse("SELECT * FROM docs LIMIT 10").unwrap();
    assert!(!query.is_match_query());
    assert!(query.is_select_query());
}

#[test]
fn test_parse_match_chain() {
    let query = Parser::parse("MATCH (a)-[:R1]->(b)-[:R2]->(c) RETURN a, c").unwrap();
    assert!(query.is_match_query());
    let mc = query.match_clause.unwrap();
    assert_eq!(mc.patterns[0].nodes.len(), 3);
    assert_eq!(mc.patterns[0].relationships.len(), 2);
}
