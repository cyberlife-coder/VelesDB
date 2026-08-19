use super::*;

#[test]
fn test_parse_simple_select() {
    let result = CoreParser::parse("SELECT * FROM documents LIMIT 10");
    assert!(result.is_ok());
    let query = result.unwrap();
    assert_eq!(query.select.from, "documents");
    assert_eq!(query.select.limit, Some(10));
}

#[test]
fn test_parse_with_where() {
    let result = CoreParser::parse("SELECT * FROM docs WHERE category = 'tech'");
    assert!(result.is_ok());
    let query = result.unwrap();
    assert!(query.select.where_clause.is_some());
}

#[test]
fn test_parse_vector_search() {
    let result = CoreParser::parse("SELECT * FROM docs WHERE vector NEAR $v LIMIT 5");
    assert!(result.is_ok());
    let query = result.unwrap();
    assert!(query.select.where_clause.is_some());
    assert_eq!(query.select.limit, Some(5));
}

#[test]
fn test_parse_invalid_query() {
    let result = CoreParser::parse("SELEC * FROM docs");
    assert!(result.is_err());
}

#[test]
fn test_parse_with_order_by() {
    let result = CoreParser::parse("SELECT * FROM docs ORDER BY created_at DESC");
    assert!(result.is_ok());
    let query = result.unwrap();
    assert!(query.select.order_by.is_some());
}

#[test]
fn test_parse_with_distinct() {
    let result = CoreParser::parse("SELECT DISTINCT category FROM products");
    assert!(result.is_ok());
    let query = result.unwrap();
    assert!(!matches!(
        query.select.distinct,
        velesdb_core::velesql::DistinctMode::None
    ));
}

#[test]
fn test_parse_with_join() {
    let result =
        CoreParser::parse("SELECT * FROM orders JOIN products ON orders.product_id = products.id");
    assert!(result.is_ok());
    let query = result.unwrap();
    assert!(!query.select.joins.is_empty());
}

#[test]
fn test_parse_with_group_by() {
    let result = CoreParser::parse("SELECT category, COUNT(*) FROM products GROUP BY category");
    assert!(result.is_ok());
    let query = result.unwrap();
    assert!(query.select.group_by.is_some());
}

#[test]
fn test_is_ddl_create_collection() {
    let query =
        CoreParser::parse("CREATE COLLECTION docs (dimension = 128, metric = 'cosine')").unwrap();
    let stmt = ParsedStatement { inner: query };
    assert!(stmt.is_ddl());
    assert!(!stmt.is_select());
    assert!(!stmt.is_dml());
}

#[test]
fn test_is_ddl_drop_collection() {
    let query = CoreParser::parse("DROP COLLECTION docs").unwrap();
    let stmt = ParsedStatement { inner: query };
    assert!(stmt.is_ddl());
    assert!(!stmt.is_select());
}

#[test]
fn test_is_delete() {
    let query = CoreParser::parse("DELETE FROM docs WHERE category = 'old'").unwrap();
    let stmt = ParsedStatement { inner: query };
    assert!(stmt.is_delete());
    assert!(stmt.is_dml());
    assert!(!stmt.is_ddl());
    assert!(!stmt.is_select());
}

#[test]
fn test_is_insert_edge() {
    let query =
        CoreParser::parse("INSERT EDGE INTO kg (source = 1, target = 2, label = 'related')")
            .unwrap();
    let stmt = ParsedStatement { inner: query };
    assert!(stmt.is_insert_edge());
    assert!(stmt.is_dml());
    assert!(!stmt.is_ddl());
}

#[test]
fn test_select_is_not_ddl_nor_dml() {
    let query = CoreParser::parse("SELECT * FROM docs LIMIT 10").unwrap();
    let stmt = ParsedStatement { inner: query };
    assert!(!stmt.is_ddl());
    assert!(!stmt.is_dml());
    assert!(!stmt.is_delete());
    assert!(!stmt.is_insert_edge());
    assert!(stmt.is_select());
}

#[test]
fn test_query_type_label_ddl() {
    let query =
        CoreParser::parse("CREATE COLLECTION docs (dimension = 128, metric = 'cosine')").unwrap();
    let stmt = ParsedStatement { inner: query };
    assert_eq!(stmt.query_type_label(), "DDL");
}

#[test]
fn test_query_type_label_dml() {
    let query = CoreParser::parse("DELETE FROM docs WHERE id = 1").unwrap();
    let stmt = ParsedStatement { inner: query };
    assert_eq!(stmt.query_type_label(), "DML");
}

#[test]
fn test_query_type_label_select() {
    let query = CoreParser::parse("SELECT * FROM docs LIMIT 10").unwrap();
    let stmt = ParsedStatement { inner: query };
    assert_eq!(stmt.query_type_label(), "SELECT");
}
