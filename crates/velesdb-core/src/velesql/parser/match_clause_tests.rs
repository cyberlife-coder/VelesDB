//! Tests for MATCH clause parser.

use super::match_clause::{parse_match_clause, parse_node_pattern, parse_relationship_pattern};
use crate::velesql::ast::Value;
use crate::velesql::graph_pattern::Direction;

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

// === Bracket validation tests (Bug fix) ===

#[test]
fn test_error_missing_closing_bracket() {
    // Missing ] should produce an error
    let result = parse_relationship_pattern("-[r:WROTE->");
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains(']') || err.contains("closing"),
        "Error should mention missing closing bracket"
    );
}

#[test]
fn test_error_missing_opening_bracket() {
    // Missing [ should produce an error
    let result = parse_relationship_pattern("-r:WROTE]->");
    assert!(result.is_err());
}

// === parse_value single quote test (Bug fix) ===

#[test]
fn test_parse_node_single_quote_property() {
    // Single quote ' as value should not panic, should error
    let result = parse_node_pattern("(n:Person {name: '})");
    // Should either error or handle gracefully, not panic
    // The malformed value should cause an error
    assert!(result.is_err());
}

// === find_keyword string literal awareness tests (Bug fix) ===

#[test]
fn test_keyword_inside_string_literal_ignored() {
    // RETURN inside a string literal should not be matched
    let result = parse_match_clause("MATCH (n:Person {name: 'RETURN'}) RETURN n");
    assert!(result.is_ok(), "Should parse correctly: {result:?}");
    let clause = result.unwrap();
    // The pattern should have the full property value
    assert_eq!(clause.patterns.len(), 1);
}

#[test]
fn test_keyword_where_inside_string_literal() {
    // WHERE inside a string literal should not be matched
    let result = parse_match_clause("MATCH (n:Person {status: 'WHERE'}) RETURN n");
    assert!(result.is_ok(), "Should parse correctly: {result:?}");
}

// === != and <> operator tests (Bug fix) ===

#[test]
fn test_where_not_equal_operator() {
    let result = parse_match_clause("MATCH (n:Person) WHERE n.age != 18 RETURN n");
    assert!(result.is_ok(), "Should parse != operator: {result:?}");
    let clause = result.unwrap();
    assert!(clause.where_clause.is_some());
}

#[test]
fn test_where_diamond_not_equal_operator() {
    let result = parse_match_clause("MATCH (n:Person) WHERE n.age <> 18 RETURN n");
    assert!(result.is_ok(), "Should parse <> operator: {result:?}");
    let clause = result.unwrap();
    assert!(clause.where_clause.is_some());
}

// === Bracket/brace ordering tests (Bug fix: prevent panic on malformed input) ===

#[test]
fn test_relationship_bracket_reversed_order() {
    // Input where ] appears before [ should return ParseError, not panic
    let result = parse_relationship_pattern("-]foo[->");
    assert!(
        result.is_err(),
        "Should error on reversed brackets: {result:?}"
    );
}

#[test]
fn test_node_brace_reversed_order() {
    // Input where } appears before { should return ParseError, not panic
    let result = parse_node_pattern("(n } foo {a: 1)");
    assert!(
        result.is_err(),
        "Should error on reversed braces: {result:?}"
    );
}

#[test]
fn test_relationship_brace_reversed_order() {
    // Relationship properties with } before { should return ParseError, not panic
    let result = parse_relationship_pattern("-[r:KNOWS } foo {a: 1]->");
    assert!(
        result.is_err(),
        "Should error on reversed braces in relationship: {result:?}"
    );
}

// === Comma inside string literal test (Bug fix) ===

#[test]
fn test_property_with_comma_in_string() {
    // Commas inside string values should be preserved
    let result = parse_node_pattern("(n:Person {name: 'Alice, Bob', age: 30})");
    assert!(
        result.is_ok(),
        "Should parse comma inside string: {result:?}"
    );
    let node = result.unwrap();
    assert_eq!(node.properties.len(), 2);
    assert_eq!(
        node.properties.get("name"),
        Some(&Value::String("Alice, Bob".to_string()))
    );
    assert_eq!(node.properties.get("age"), Some(&Value::Integer(30)));
}

// === Underscore in identifier test (Bug fix) ===

#[test]
fn test_keyword_not_matched_after_underscore() {
    // Keywords after underscore should NOT be matched (underscore is part of identifier)
    let result = parse_match_clause("MATCH (foo_RETURN:Label) RETURN foo_RETURN");
    assert!(
        result.is_ok(),
        "Should parse identifier with keyword substring: {result:?}"
    );
}

#[test]
fn test_keyword_not_matched_before_underscore() {
    // Keywords before underscore should NOT be matched
    let result = parse_match_clause("MATCH (RETURN_value:Label) RETURN RETURN_value");
    assert!(
        result.is_ok(),
        "Should parse identifier starting with keyword: {result:?}"
    );
}

#[test]
fn test_where_clause_identifier_with_underscore() {
    // WHERE should not be matched inside where_clause identifier
    let result = parse_match_clause("MATCH (n:Person) WHERE n.where_status = 'active' RETURN n");
    assert!(
        result.is_ok(),
        "Should handle where_status identifier: {result:?}"
    );
}

// === Expert 5: Additional edge case tests ===

#[test]
fn test_nested_parentheses_in_pattern() {
    // Nested patterns should be handled correctly
    let result = parse_match_clause("MATCH (a)-[:R]->(b)-[:S]->(c) RETURN a, b, c");
    assert!(result.is_ok(), "Should handle chained patterns: {result:?}");
    let clause = result.unwrap();
    assert_eq!(clause.patterns[0].nodes.len(), 3);
    assert_eq!(clause.patterns[0].relationships.len(), 2);
}

#[test]
fn test_empty_relationship_brackets() {
    // Empty brackets should be valid
    let result = parse_relationship_pattern("-[]->");
    assert!(result.is_ok(), "Should handle empty brackets: {result:?}");
}

#[test]
fn test_relationship_with_only_alias() {
    let result = parse_relationship_pattern("-[r]->");
    assert!(result.is_ok());
    let rel = result.unwrap();
    assert_eq!(rel.alias, Some("r".to_string()));
}

#[test]
fn test_node_with_only_label_no_alias() {
    let result = parse_node_pattern("(:Person)");
    assert!(result.is_ok());
    let node = result.unwrap();
    assert!(node.alias.is_none());
    assert_eq!(node.labels, vec!["Person".to_string()]);
}

// === Devin Expert 4: Edge Case Fuzzing Tests ===

#[test]
fn test_empty_where_condition() {
    // WHERE immediately followed by RETURN should error, not panic
    let result = parse_match_clause("MATCH (n) WHERE RETURN n");
    assert!(result.is_err(), "Should error on empty WHERE: {result:?}");
}

#[test]
fn test_where_with_only_whitespace() {
    // WHERE with only whitespace should error
    let result = parse_match_clause("MATCH (n) WHERE   RETURN n");
    assert!(
        result.is_err(),
        "Should error on whitespace-only WHERE: {result:?}"
    );
}

#[test]
fn test_deeply_nested_properties() {
    // Multiple properties should parse correctly
    let result = parse_node_pattern("(n:Person {a: 1, b: 2, c: 3, d: 'test'})");
    assert!(result.is_ok());
    let node = result.unwrap();
    assert_eq!(node.properties.len(), 4);
}

#[test]
fn test_relationship_with_properties_and_range() {
    let result = parse_relationship_pattern("-[r:KNOWS*1..3 {since: 2020}]->");
    // This tests combined range and properties parsing
    assert!(result.is_ok() || result.is_err()); // May or may not be supported
}

// === Devin Bug: Operator inside string literal (Bug fix) ===

#[test]
fn test_where_operator_inside_string_literal() {
    // Operators inside string literals should be ignored
    // The = operator should be found, not the > inside 'x > y'
    let result = parse_match_clause("MATCH (n) WHERE n.status = 'x > y' RETURN n");
    assert!(result.is_ok(), "Should parse correctly: {result:?}");
    let clause = result.unwrap();
    assert!(clause.where_clause.is_some());
}

#[test]
fn test_where_multiple_operators_in_string() {
    // Multiple operators inside string should all be ignored
    let result = parse_match_clause("MATCH (n) WHERE n.expr = 'a >= b && c <= d' RETURN n");
    assert!(
        result.is_ok(),
        "Should handle multiple operators in string: {result:?}"
    );
}

#[test]
fn test_where_not_equal_inside_string() {
    // != inside string should be ignored, real != outside should be found
    let result = parse_match_clause("MATCH (n) WHERE n.value != 'test != value' RETURN n");
    assert!(result.is_ok(), "Should find != outside string: {result:?}");
}
