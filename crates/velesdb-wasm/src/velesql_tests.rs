use velesdb_core::velesql::Parser;

// === MATCH Query Tests (EPIC-053 US-004) ===

#[test]
fn test_parse_match_query() {
    let parsed = Parser::parse("MATCH (p:Person)-[:KNOWS]->(f:Person) RETURN f.name");
    assert!(parsed.is_ok(), "MATCH query should parse: {parsed:?}");
    let query = parsed.unwrap();
    assert!(query.is_match_query());
    assert!(!query.is_select_query());
}

#[test]
fn test_match_query_node_count() {
    let parsed = Parser::parse("MATCH (p:Person)-[:KNOWS]->(f:Person) RETURN f.name").unwrap();
    let mc = parsed
        .match_clause
        .as_ref()
        .expect("should have match_clause");
    assert_eq!(mc.patterns[0].nodes.len(), 2);
}

#[test]
fn test_match_query_relationship_count() {
    let parsed = Parser::parse("MATCH (a)-[:REL1]->(b)-[:REL2]->(c) RETURN a, b, c").unwrap();
    let mc = parsed
        .match_clause
        .as_ref()
        .expect("should have match_clause");
    assert_eq!(mc.patterns[0].relationships.len(), 2);
}

#[test]
fn test_match_query_node_labels() {
    let parsed = Parser::parse("MATCH (p:Person:Author) RETURN p").unwrap();
    let mc = parsed
        .match_clause
        .as_ref()
        .expect("should have match_clause");
    let node = &mc.patterns[0].nodes[0];
    assert!(node.labels.contains(&"Person".to_string()));
}

#[test]
fn test_match_query_relationship_types() {
    let parsed = Parser::parse("MATCH (a)-[:WROTE]->(b) RETURN b").unwrap();
    let mc = parsed
        .match_clause
        .as_ref()
        .expect("should have match_clause");
    let rel = &mc.patterns[0].relationships[0];
    assert!(rel.types.contains(&"WROTE".to_string()));
}

#[test]
fn test_match_query_without_where() {
    // MATCH queries without WHERE work correctly
    let parsed = Parser::parse("MATCH (p:Person) RETURN p.name").unwrap();
    let mc = parsed
        .match_clause
        .as_ref()
        .expect("should have match_clause");
    assert!(mc.where_clause.is_none());
}

#[test]
fn test_match_query_return_items() {
    let parsed = Parser::parse("MATCH (p:Person) RETURN p.name, p.age AS years").unwrap();
    let mc = parsed
        .match_clause
        .as_ref()
        .expect("should have match_clause");
    assert_eq!(mc.return_clause.items.len(), 2);
    assert_eq!(mc.return_clause.items[1].alias, Some("years".to_string()));
}

#[test]
fn test_match_query_limit() {
    let parsed = Parser::parse("MATCH (p:Person) RETURN p LIMIT 10").unwrap();
    let mc = parsed
        .match_clause
        .as_ref()
        .expect("should have match_clause");
    assert_eq!(mc.return_clause.limit, Some(10));
}

// === DDL/DML Introspection Tests (VelesQL v3.3) ===

#[test]
fn test_ddl_create_collection_detected() {
    let query =
        Parser::parse("CREATE COLLECTION docs (dimension = 768, metric = 'cosine');").unwrap();
    assert!(query.is_ddl_query());
    assert!(!query.is_dml_query());
    assert!(!query.is_select_query());
}

#[test]
fn test_ddl_drop_collection_detected() {
    let query = Parser::parse("DROP COLLECTION docs;").unwrap();
    assert!(query.is_ddl_query());
    assert!(!query.is_dml_query());
}

#[test]
fn test_dml_insert_edge_detected() {
    let query =
        Parser::parse("INSERT EDGE INTO kg (source = 1, target = 2, label = 'KNOWS');").unwrap();
    assert!(query.is_dml_query());
    assert!(!query.is_ddl_query());
    assert!(!query.is_select_query());
    assert!(matches!(
        &query.dml,
        Some(velesdb_core::velesql::DmlStatement::InsertEdge(_))
    ));
}

#[test]
fn test_dml_delete_detected() {
    let query = Parser::parse("DELETE FROM docs WHERE id = 1;").unwrap();
    assert!(query.is_dml_query());
    assert!(!query.is_ddl_query());
    assert!(matches!(
        &query.dml,
        Some(velesdb_core::velesql::DmlStatement::Delete(_))
    ));
}

#[test]
fn test_dml_delete_edge_detected() {
    let query = Parser::parse("DELETE EDGE 42 FROM kg;").unwrap();
    assert!(query.is_dml_query());
    assert!(matches!(
        &query.dml,
        Some(velesdb_core::velesql::DmlStatement::DeleteEdge(_))
    ));
}

#[test]
fn test_select_is_not_ddl_or_dml() {
    let query = Parser::parse("SELECT * FROM docs LIMIT 10").unwrap();
    assert!(!query.is_ddl_query());
    assert!(!query.is_dml_query());
    assert!(query.is_select_query());
}

// === Original SELECT Tests ===

#[test]
fn test_parse_simple_select() {
    let parsed = Parser::parse("SELECT * FROM documents LIMIT 10");
    assert!(parsed.is_ok());
    let query = parsed.unwrap();
    assert!(query.is_select_query());
    assert!(!query.is_match_query());
    assert_eq!(query.select.from, "documents");
    assert_eq!(query.select.limit, Some(10));
}

#[test]
fn test_parse_invalid_query() {
    let parsed = Parser::parse("SELEC * FROM docs");
    assert!(parsed.is_err());
}

#[test]
fn test_is_valid() {
    assert!(Parser::parse("SELECT * FROM docs").is_ok());
    assert!(Parser::parse("SELECT id FROM docs WHERE x = 1").is_ok());
    assert!(Parser::parse("SELEC * FROM docs").is_err());
}

#[test]
fn test_parse_with_where() {
    let parsed = Parser::parse("SELECT * FROM docs WHERE category = 'tech'").unwrap();
    assert!(parsed.select.where_clause.is_some());
}

#[test]
fn test_parse_vector_search() {
    let parsed = Parser::parse("SELECT * FROM docs WHERE vector NEAR $v LIMIT 10").unwrap();
    assert!(parsed.select.where_clause.is_some());
}

#[test]
fn test_parse_with_order_by() {
    let parsed = Parser::parse("SELECT * FROM docs ORDER BY created_at DESC").unwrap();
    assert!(parsed.select.order_by.is_some());
}

#[test]
fn test_parse_with_join() {
    let parsed =
        Parser::parse("SELECT * FROM orders JOIN products ON orders.product_id = products.id")
            .unwrap();
    assert!(!parsed.select.joins.is_empty());
    assert_eq!(parsed.select.joins.len(), 1);
}

#[test]
fn test_parse_with_group_by() {
    let parsed =
        Parser::parse("SELECT category, COUNT(*) FROM products GROUP BY category").unwrap();
    assert!(parsed.select.group_by.is_some());
}
