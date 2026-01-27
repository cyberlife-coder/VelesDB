//! Tests for subquery expression parsing (EPIC-039).

use crate::velesql::ast::Value;
use crate::velesql::Parser;

#[test]
fn test_parse_simple_scalar_subquery() {
    let query = "SELECT * FROM products WHERE price < (SELECT AVG(price) FROM products)";
    let result = Parser::parse(query);
    assert!(
        result.is_ok(),
        "Failed to parse scalar subquery: {:?}",
        result.err()
    );
}

#[test]
fn test_parse_subquery_with_where() {
    let query =
        "SELECT * FROM orders WHERE total > (SELECT AVG(total) FROM orders WHERE status = 'paid')";
    let result = Parser::parse(query);
    assert!(
        result.is_ok(),
        "Failed to parse subquery with WHERE: {:?}",
        result.err()
    );
}

#[test]
fn test_parse_subquery_with_aggregation() {
    let query = "SELECT * FROM accounts WHERE balance > (SELECT SUM(amount) FROM transactions)";
    let result = Parser::parse(query);
    assert!(
        result.is_ok(),
        "Failed to parse subquery with aggregation: {:?}",
        result.err()
    );
}

#[test]
fn test_parse_subquery_count() {
    // Note: subquery must be on the right side of comparison
    let query = "SELECT * FROM users WHERE order_count > (SELECT COUNT(*) FROM orders)";
    let result = Parser::parse(query);
    assert!(
        result.is_ok(),
        "Failed to parse COUNT subquery: {:?}",
        result.err()
    );
}

#[test]
fn test_parse_subquery_min_max() {
    let query = "SELECT * FROM products WHERE price = (SELECT MIN(price) FROM products)";
    let result = Parser::parse(query);
    assert!(
        result.is_ok(),
        "Failed to parse MIN subquery: {:?}",
        result.err()
    );
}

#[test]
fn test_subquery_value_variant() {
    let query = "SELECT * FROM items WHERE cost < (SELECT AVG(cost) FROM items)";
    let result = Parser::parse(query);
    assert!(result.is_ok());

    let parsed = result.unwrap();
    if let Some(crate::velesql::Condition::Comparison(cmp)) = parsed.select.where_clause.as_ref() {
        assert!(
            matches!(cmp.value, Value::Subquery(_)),
            "Expected Subquery value, got {:?}",
            cmp.value
        );
    } else {
        panic!("Expected Comparison condition with subquery");
    }
}

#[test]
fn test_parse_subquery_with_limit() {
    let query = "SELECT * FROM logs WHERE id > (SELECT MAX(id) FROM logs LIMIT 1)";
    let result = Parser::parse(query);
    assert!(
        result.is_ok(),
        "Failed to parse subquery with LIMIT: {:?}",
        result.err()
    );
}
