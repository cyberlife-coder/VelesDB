use super::*;

#[test]
fn test_detect_query_type_search() {
    let parsed =
        velesql::Parser::parse("SELECT * FROM docs WHERE similarity(embedding, $v) > 0.8 LIMIT 10")
            .unwrap();
    assert_eq!(detect_query_type(&parsed), QueryType::Search);
}

#[test]
fn test_detect_query_type_aggregation() {
    let parsed =
        velesql::Parser::parse("SELECT category, COUNT(*) FROM products GROUP BY category")
            .unwrap();
    assert_eq!(detect_query_type(&parsed), QueryType::Aggregation);
}

#[test]
fn test_detect_query_type_rows() {
    let parsed =
        velesql::Parser::parse("SELECT name, price FROM products WHERE price > 100").unwrap();
    assert_eq!(detect_query_type(&parsed), QueryType::Rows);
}

#[test]
fn test_detect_query_type_graph() {
    let parsed =
        velesql::Parser::parse("MATCH (n:Person)-[:KNOWS]->(m) RETURN n.name, m.name LIMIT 10")
            .unwrap();
    assert_eq!(detect_query_type(&parsed), QueryType::Graph);
}

#[test]
fn test_detect_query_type_hybrid_vector_aggregation() {
    // When both vector search and aggregation, aggregation takes priority
    let parsed = velesql::Parser::parse(
        "SELECT category, COUNT(*) FROM docs WHERE similarity(embedding, $v) > 0.7 GROUP BY category",
    )
    .unwrap();
    assert_eq!(detect_query_type(&parsed), QueryType::Aggregation);
}

#[test]
fn test_detect_query_type_ddl_create() {
    let parsed =
        velesql::Parser::parse("CREATE COLLECTION docs (dimension = 768, metric = 'cosine');")
            .unwrap();
    assert_eq!(detect_query_type(&parsed), QueryType::Ddl);
}

#[test]
fn test_detect_query_type_ddl_drop() {
    let parsed = velesql::Parser::parse("DROP COLLECTION docs;").unwrap();
    assert_eq!(detect_query_type(&parsed), QueryType::Ddl);
}

#[test]
fn test_detect_query_type_dml_insert_edge() {
    let parsed =
        velesql::Parser::parse("INSERT EDGE INTO kg (source = 1, target = 2, label = 'KNOWS');")
            .unwrap();
    assert_eq!(detect_query_type(&parsed), QueryType::Dml);
}

#[test]
fn test_detect_query_type_dml_delete() {
    let parsed = velesql::Parser::parse("DELETE FROM docs WHERE id = 1;").unwrap();
    assert_eq!(detect_query_type(&parsed), QueryType::Dml);
}
