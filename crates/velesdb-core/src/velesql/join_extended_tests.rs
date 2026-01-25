//! Tests for extended JOIN support (EPIC-040 US-003).
//!
//! Covers:
//! - LEFT/RIGHT/FULL JOIN types
//! - Table aliases
//! - USING clause

use crate::velesql::{JoinType, Parser};

#[test]
fn test_inner_join_explicit() {
    let sql = "SELECT * FROM orders INNER JOIN customers ON orders.customer_id = customers.id";
    let result = Parser::parse(sql);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let query = result.unwrap();
    assert_eq!(query.select.joins.len(), 1);
    assert_eq!(query.select.joins[0].join_type, JoinType::Inner);
    assert_eq!(query.select.joins[0].table, "customers");
}

#[test]
fn test_left_join() {
    let sql = "SELECT * FROM orders LEFT JOIN customers ON orders.customer_id = customers.id";
    let result = Parser::parse(sql);
    assert!(
        result.is_ok(),
        "Failed to parse LEFT JOIN: {:?}",
        result.err()
    );

    let query = result.unwrap();
    assert_eq!(query.select.joins.len(), 1);
    assert_eq!(query.select.joins[0].join_type, JoinType::Left);
}

#[test]
fn test_left_outer_join() {
    let sql = "SELECT * FROM orders LEFT OUTER JOIN customers ON orders.customer_id = customers.id";
    let result = Parser::parse(sql);
    assert!(
        result.is_ok(),
        "Failed to parse LEFT OUTER JOIN: {:?}",
        result.err()
    );

    let query = result.unwrap();
    assert_eq!(query.select.joins[0].join_type, JoinType::Left);
}

#[test]
fn test_right_join() {
    let sql = "SELECT * FROM orders RIGHT JOIN customers ON orders.customer_id = customers.id";
    let result = Parser::parse(sql);
    assert!(
        result.is_ok(),
        "Failed to parse RIGHT JOIN: {:?}",
        result.err()
    );

    let query = result.unwrap();
    assert_eq!(query.select.joins[0].join_type, JoinType::Right);
}

#[test]
fn test_full_join() {
    let sql = "SELECT * FROM orders FULL JOIN customers ON orders.customer_id = customers.id";
    let result = Parser::parse(sql);
    assert!(
        result.is_ok(),
        "Failed to parse FULL JOIN: {:?}",
        result.err()
    );

    let query = result.unwrap();
    assert_eq!(query.select.joins[0].join_type, JoinType::Full);
}

#[test]
fn test_full_outer_join() {
    let sql = "SELECT * FROM orders FULL OUTER JOIN customers ON orders.customer_id = customers.id";
    let result = Parser::parse(sql);
    assert!(
        result.is_ok(),
        "Failed to parse FULL OUTER JOIN: {:?}",
        result.err()
    );

    let query = result.unwrap();
    assert_eq!(query.select.joins[0].join_type, JoinType::Full);
}

#[test]
fn test_join_with_alias() {
    // Note: FROM table alias not yet supported, only JOIN alias
    let sql = "SELECT * FROM docs JOIN meta AS m ON docs.id = m.doc_id";
    let result = Parser::parse(sql);
    assert!(
        result.is_ok(),
        "Failed to parse JOIN with alias: {:?}",
        result.err()
    );

    let query = result.unwrap();
    assert_eq!(query.select.joins[0].table, "meta");
    assert_eq!(query.select.joins[0].alias, Some("m".to_string()));
}

#[test]
fn test_join_using_clause() {
    let sql = "SELECT * FROM orders JOIN customers USING (customer_id)";
    let result = Parser::parse(sql);
    assert!(
        result.is_ok(),
        "Failed to parse JOIN USING: {:?}",
        result.err()
    );

    let query = result.unwrap();
    assert!(query.select.joins[0].using_columns.is_some());
    let using = query.select.joins[0].using_columns.as_ref().unwrap();
    assert_eq!(using, &vec!["customer_id".to_string()]);
}

#[test]
fn test_join_using_multiple_columns() {
    let sql = "SELECT * FROM orders JOIN customers USING (customer_id, region_id)";
    let result = Parser::parse(sql);
    assert!(
        result.is_ok(),
        "Failed to parse JOIN USING multiple: {:?}",
        result.err()
    );

    let query = result.unwrap();
    let using = query.select.joins[0].using_columns.as_ref().unwrap();
    assert_eq!(using.len(), 2);
    assert_eq!(using[0], "customer_id");
    assert_eq!(using[1], "region_id");
}

#[test]
fn test_multiple_joins_mixed_types() {
    let sql = "SELECT * FROM orders LEFT JOIN customers ON orders.cid = customers.id RIGHT JOIN products ON orders.pid = products.id";
    let result = Parser::parse(sql);
    assert!(
        result.is_ok(),
        "Failed to parse multiple joins: {:?}",
        result.err()
    );

    let query = result.unwrap();
    assert_eq!(query.select.joins.len(), 2);
    assert_eq!(query.select.joins[0].join_type, JoinType::Left);
    assert_eq!(query.select.joins[1].join_type, JoinType::Right);
}

#[test]
fn test_default_join_is_inner() {
    // Plain JOIN without type specifier should be INNER
    let sql = "SELECT * FROM orders JOIN customers ON orders.customer_id = customers.id";
    let result = Parser::parse(sql);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let query = result.unwrap();
    assert_eq!(query.select.joins[0].join_type, JoinType::Inner);
}
