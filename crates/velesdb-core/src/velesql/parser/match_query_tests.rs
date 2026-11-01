//! Tests for MATCH query parsing (EPIC-045 US-001).

use crate::velesql::ast::Value;
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
    let query = Parser::parse("MATCH (n:Person) WHERE n.age > 18 RETURN n").unwrap();
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

// === Behaviours the removed string-scanning MATCH parser was tested for ===
//
// `velesql::match_clause` parsed MATCH by scanning strings, beside the pest
// grammar every query goes through; only its own tests called it. These are
// the behaviours those tests checked, held against the production parser.

fn match_clause(query: &str) -> crate::velesql::graph_pattern::MatchClause {
    Parser::parse(query)
        .unwrap_or_else(|e| panic!("{query}: {e}"))
        .match_clause
        .unwrap_or_else(|| panic!("{query}: not a MATCH query"))
}

fn first_rel(query: &str) -> crate::velesql::graph_pattern::RelationshipPattern {
    match_clause(query).patterns[0].relationships[0].clone()
}

fn assert_rejected(query: &str) {
    assert!(Parser::parse(query).is_err(), "{query} must not parse");
}

#[test]
fn match_anonymous_and_label_only_nodes_have_no_alias() {
    let mc = match_clause("MATCH (a)-[:R]->() RETURN a");
    assert!(mc.patterns[0].nodes[1].alias.is_none());
    let mc = match_clause("MATCH (:Person)-[:R]->(b) RETURN b");
    let node = &mc.patterns[0].nodes[0];
    assert!(node.alias.is_none());
    assert_eq!(node.labels, vec!["Person".to_string()]);
}

#[test]
fn match_relationship_type_alternatives() {
    let rel = first_rel("MATCH (a)-[:WROTE|CREATED]->(b) RETURN a");
    assert_eq!(rel.types, vec!["WROTE".to_string(), "CREATED".to_string()]);
}

#[test]
fn match_relationship_alias_only_and_empty_brackets() {
    let rel = first_rel("MATCH (a)-[r]->(b) RETURN a");
    assert_eq!(rel.alias, Some("r".to_string()));
    assert!(rel.types.is_empty());
    let rel = first_rel("MATCH (a)-[]->(b) RETURN a");
    assert_eq!(rel.direction, Direction::Outgoing);
    assert!(rel.alias.is_none());
}

#[test]
fn match_variable_length_range_forms() {
    for (pattern, range) in [("*3", (3, 3)), ("*..5", (1, 5))] {
        let rel = first_rel(&format!("MATCH (a)-[{pattern}]->(b) RETURN a"));
        assert_eq!(rel.range, Some(range), "[{pattern}]");
    }
}

/// An open upper bound parses to `u32::MAX` hops, which validation refuses
/// as a graph expansion past its limit rather than running unbounded.
#[test]
fn match_unbounded_variable_length_is_refused() {
    for pattern in ["*", "*2.."] {
        let err = Parser::parse(&format!("MATCH (a)-[{pattern}]->(b) RETURN a"))
            .expect_err("an unbounded expansion must be refused");
        assert!(err.to_string().contains("E007"), "[{pattern}]: {err}");
    }
}

#[test]
fn match_relationship_with_type_range_and_properties() {
    let rel = first_rel("MATCH (a)-[r:KNOWS*1..3 {since: 2020}]->(b) RETURN a");
    assert_eq!(rel.direction, Direction::Outgoing);
    assert_eq!(rel.alias.as_deref(), Some("r"));
    assert_eq!(rel.types, vec!["KNOWS".to_string()]);
    assert_eq!(rel.range, Some((1, 3)));
    assert_eq!(rel.properties.get("since"), Some(&Value::Integer(2020)));
}

#[test]
fn match_node_properties_keep_commas_and_count() {
    let mc = match_clause("MATCH (n:Person {name: 'Alice, Bob', age: 30}) RETURN n");
    let props = &mc.patterns[0].nodes[0].properties;
    assert_eq!(props.len(), 2);
    assert_eq!(
        props.get("name"),
        Some(&Value::String("Alice, Bob".to_string()))
    );
    assert_eq!(props.get("age"), Some(&Value::Integer(30)));

    let mc = match_clause("MATCH (n:Person {a: 1, b: 2, c: 3, d: 'test'}) RETURN n");
    assert_eq!(mc.patterns[0].nodes[0].properties.len(), 4);
}

#[test]
fn match_keywords_inside_string_literals_are_values() {
    for query in [
        "MATCH (n:Person {name: 'RETURN'}) RETURN n",
        "MATCH (n:Person {status: 'WHERE'}) RETURN n",
    ] {
        assert_eq!(match_clause(query).patterns.len(), 1, "{query}");
    }
}

#[test]
fn match_identifiers_containing_keywords() {
    for query in [
        "MATCH (foo_RETURN:Label) RETURN foo_RETURN",
        "MATCH (RETURN_value:Label) RETURN RETURN_value",
        "MATCH (n:Person) WHERE n.where_status = 'active' RETURN n",
    ] {
        match_clause(query);
    }
}

#[test]
fn match_where_comparison_operators_outside_string_literals() {
    for query in [
        "MATCH (n:Person) WHERE n.age != 18 RETURN n",
        "MATCH (n:Person) WHERE n.age <> 18 RETURN n",
        "MATCH (n) WHERE n.status = 'x > y' RETURN n",
        "MATCH (n) WHERE n.expr = 'a >= b && c <= d' RETURN n",
        "MATCH (n) WHERE n.value != 'test != value' RETURN n",
    ] {
        assert!(match_clause(query).where_clause.is_some(), "{query}");
    }
}

#[test]
fn match_malformed_queries_are_rejected() {
    for query in [
        "MATCH (n:Person)",
        "MATCH RETURN n",
        "MATCH (n:Person RETURN n",
        "MATCH (a)<<-[r:WROTE]->>(b) RETURN a",
        "MATCH (a)-[r:WROTE->(b) RETURN a",
        "MATCH (a)-r:WROTE]->(b) RETURN a",
        "MATCH (a)-]foo[->(b) RETURN a",
        "MATCH (n } foo {a: 1) RETURN n",
        "MATCH (a)-[r:KNOWS } foo {a: 1]->(b) RETURN a",
        "MATCH (n:Person {name: '}) RETURN n",
        "MATCH (n) WHERE RETURN n",
        "MATCH (n) WHERE   RETURN n",
    ] {
        assert_rejected(query);
    }
}

#[test]
fn match_property_literals_take_their_value_types() {
    let mc = match_clause(
        "MATCH (n {s: 'hello', i: 42, f: 2.72, t: true, f2: FALSE, z: null}) RETURN n",
    );
    let props = &mc.patterns[0].nodes[0].properties;
    assert_eq!(props.get("s"), Some(&Value::String("hello".to_string())));
    assert_eq!(props.get("i"), Some(&Value::Integer(42)));
    assert_eq!(props.get("f"), Some(&Value::Float(2.72)));
    assert_eq!(props.get("t"), Some(&Value::Boolean(true)));
    assert_eq!(props.get("f2"), Some(&Value::Boolean(false)));
    assert_eq!(props.get("z"), Some(&Value::Null));
}
