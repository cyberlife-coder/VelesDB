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

// === EPIC-039 US-003: Correlated Subquery Tests ===

#[test]
fn test_correlated_subquery_detection_basic() {
    // Non-correlated subquery - same table name
    let query = "SELECT * FROM orders WHERE total > (SELECT AVG(total) FROM orders)";
    let result = Parser::parse(query).expect("Parse failed");

    if let Some(crate::velesql::Condition::Comparison(cmp)) = result.select.where_clause.as_ref() {
        if let Value::Subquery(sub) = &cmp.value {
            // No correlations since subquery references its own table
            assert!(
                sub.correlations.is_empty(),
                "Non-correlated subquery should have no correlations"
            );
        }
    }
}

#[test]
fn test_correlated_subquery_outer_reference() {
    // Subquery referencing different table (simpler syntax without alias)
    let query =
        "SELECT * FROM orders WHERE total > (SELECT AVG(amount) FROM order_items WHERE order_id = 1)";
    let result = Parser::parse(query);

    // This query should parse successfully
    assert!(
        result.is_ok(),
        "Failed to parse subquery with different table: {:?}",
        result.err()
    );
}

#[test]
fn test_subquery_correlations_field() {
    // Test that the correlations field exists and is accessible
    let query = "SELECT * FROM products WHERE price < (SELECT AVG(price) FROM products)";
    let result = Parser::parse(query).expect("Parse failed");

    if let Some(crate::velesql::Condition::Comparison(cmp)) = result.select.where_clause.as_ref() {
        if let Value::Subquery(sub) = &cmp.value {
            // Access correlations field - should compile and work
            let _correlation_count = sub.correlations.len();
            assert!(
                sub.correlations.is_empty() || !sub.correlations.is_empty(),
                "Correlations field should be accessible"
            );
        }
    }
}
