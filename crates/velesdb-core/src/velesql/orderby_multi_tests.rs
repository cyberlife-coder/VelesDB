//! Tests for ORDER BY multi-expression support (EPIC-040 US-002).
//!
//! Covers:
//! - ORDER BY multiple columns
//! - ORDER BY with aggregate functions
//! - ORDER BY mixed (columns + aggregates)
//! - Direction per column (ASC/DESC)

use crate::velesql::{OrderByExpr, Parser};

#[test]
fn test_orderby_multiple_columns() {
    let sql = "SELECT * FROM products ORDER BY category ASC, price DESC";
    let result = Parser::parse(sql);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let query = result.unwrap();
    let order_by = query.select.order_by.expect("ORDER BY should be present");

    assert_eq!(order_by.len(), 2, "Should have 2 ORDER BY items");
    // First: category ASC
    assert!(matches!(&order_by[0].expr, OrderByExpr::Field(f) if f == "category"));
    assert!(!order_by[0].descending, "category should be ASC");
    // Second: price DESC
    assert!(matches!(&order_by[1].expr, OrderByExpr::Field(f) if f == "price"));
    assert!(order_by[1].descending, "price should be DESC");
}

#[test]
fn test_orderby_with_aggregate() {
    let sql = "SELECT category, COUNT(*) FROM products GROUP BY category ORDER BY COUNT(*) DESC";
    let result = Parser::parse(sql);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let query = result.unwrap();
    let order_by = query.select.order_by.expect("ORDER BY should be present");

    assert_eq!(order_by.len(), 1);
    assert!(order_by[0].descending, "Should be DESC");
    // The expression should represent COUNT(*)
    assert!(
        matches!(&order_by[0].expr, OrderByExpr::Aggregate(_)),
        "Should be Aggregate variant"
    );
}

#[test]
fn test_orderby_mixed_columns_and_aggregates() {
    let sql = "SELECT category, COUNT(*), AVG(price) FROM products GROUP BY category ORDER BY COUNT(*) DESC, category ASC";
    let result = Parser::parse(sql);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let query = result.unwrap();
    let order_by = query.select.order_by.expect("ORDER BY should be present");

    assert_eq!(order_by.len(), 2, "Should have 2 ORDER BY items");
    // First: COUNT(*) DESC
    assert!(order_by[0].descending);
    // Second: category ASC
    assert!(!order_by[1].descending);
}

#[test]
fn test_orderby_aggregate_with_column_arg() {
    let sql =
        "SELECT category, SUM(price) FROM products GROUP BY category ORDER BY SUM(price) DESC";
    let result = Parser::parse(sql);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let query = result.unwrap();
    let order_by = query.select.order_by.expect("ORDER BY should be present");

    assert_eq!(order_by.len(), 1);
    assert!(order_by[0].descending);
}

#[test]
fn test_orderby_default_direction_is_asc() {
    let sql = "SELECT * FROM products ORDER BY price, category";
    let result = Parser::parse(sql);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let query = result.unwrap();
    let order_by = query.select.order_by.expect("ORDER BY should be present");

    assert_eq!(order_by.len(), 2);
    // Both should default to ASC (descending = false)
    assert!(matches!(&order_by[0].expr, OrderByExpr::Field(f) if f == "price"));
    assert!(!order_by[0].descending, "Default should be ASC");
    assert!(matches!(&order_by[1].expr, OrderByExpr::Field(f) if f == "category"));
    assert!(!order_by[1].descending, "Default should be ASC");
}

#[test]
fn test_orderby_similarity_with_column() {
    let sql = "SELECT * FROM products ORDER BY similarity(embedding, $query) DESC, price ASC";
    let result = Parser::parse(sql);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let query = result.unwrap();
    let order_by = query.select.order_by.expect("ORDER BY should be present");

    assert_eq!(order_by.len(), 2);
    assert!(matches!(&order_by[0].expr, OrderByExpr::Similarity(_)));
    assert!(order_by[0].descending, "similarity should be DESC");
    assert!(matches!(&order_by[1].expr, OrderByExpr::Field(f) if f == "price"));
    assert!(!order_by[1].descending, "price should be ASC");
}
