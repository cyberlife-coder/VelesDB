//! PR Review Bugfix Tests - TDD approach (Martin Fowler)
//!
//! Each bug gets a failing test FIRST, then we fix the code.
//! Tests are named: test_bug_XX_description

use crate::velesql::ast::Value;
use crate::velesql::{LogicalOp, Parser};

// =============================================================================
// BUG 10: Parser/grammar allow SUM(*)/AVG(*) leading to silently null aggregates
// Only COUNT(*) is semantically valid, others need a column name
// =============================================================================

#[test]
fn test_bug_10_sum_star_should_fail() {
    // SUM(*) doesn't make sense - you can't sum all columns
    let sql = "SELECT SUM(*) FROM products";
    let result = Parser::parse(sql);

    // This SHOULD fail or return an error, but currently parses successfully
    // After fix: result.is_err() should be true
    assert!(
        result.is_err(),
        "SUM(*) should be rejected - only COUNT(*) is valid with *. Got: {:?}",
        result
    );
}

#[test]
fn test_bug_10_avg_star_should_fail() {
    // AVG(*) doesn't make sense
    let sql = "SELECT AVG(*) FROM products";
    let result = Parser::parse(sql);

    assert!(
        result.is_err(),
        "AVG(*) should be rejected - only COUNT(*) is valid with *. Got: {:?}",
        result
    );
}

#[test]
fn test_bug_10_min_star_should_fail() {
    let sql = "SELECT MIN(*) FROM products";
    let result = Parser::parse(sql);

    assert!(
        result.is_err(),
        "MIN(*) should be rejected - only COUNT(*) is valid with *"
    );
}

#[test]
fn test_bug_10_max_star_should_fail() {
    let sql = "SELECT MAX(*) FROM products";
    let result = Parser::parse(sql);

    assert!(
        result.is_err(),
        "MAX(*) should be rejected - only COUNT(*) is valid with *"
    );
}

#[test]
fn test_bug_10_count_star_should_succeed() {
    // COUNT(*) IS valid
    let sql = "SELECT COUNT(*) FROM products";
    let result = Parser::parse(sql);

    assert!(
        result.is_ok(),
        "COUNT(*) should parse successfully: {:?}",
        result.err()
    );
}

#[test]
fn test_bug_10_sum_column_should_succeed() {
    // SUM(column) is valid
    let sql = "SELECT SUM(price) FROM products";
    let result = Parser::parse(sql);

    assert!(
        result.is_ok(),
        "SUM(column) should parse successfully: {:?}",
        result.err()
    );
}

// =============================================================================
// BUG 6: Grammar doesn't emit AND/OR tokens in HAVING clause
// The grammar has the rules but pest might not emit them as separate tokens
// =============================================================================

#[test]
fn test_bug_6_having_or_tokens_captured() {
    let sql = "SELECT category, COUNT(*) FROM products GROUP BY category HAVING COUNT(*) > 5 OR COUNT(*) < 100";
    let result = Parser::parse(sql);

    assert!(
        result.is_ok(),
        "Failed to parse HAVING with OR: {:?}",
        result.err()
    );

    let query = result.unwrap();
    let having = query
        .select
        .having
        .as_ref()
        .expect("HAVING clause should exist");

    // After fix: operators should contain LogicalOp::Or
    assert!(
        !having.operators.is_empty(),
        "HAVING operators should be captured. Got empty operators vec. Conditions: {:?}",
        having.conditions
    );

    assert_eq!(
        having.operators.len(),
        1,
        "Should have 1 operator for 2 conditions. Got: {:?}",
        having.operators
    );
    assert!(
        matches!(having.operators[0], LogicalOp::Or),
        "Expected OR operator, got: {:?}",
        having.operators[0]
    );
}

#[test]
fn test_bug_6_having_and_tokens_captured() {
    let sql = "SELECT category, COUNT(*) FROM products GROUP BY category HAVING COUNT(*) > 5 AND AVG(price) < 100";
    let result = Parser::parse(sql);

    assert!(
        result.is_ok(),
        "Failed to parse HAVING with AND: {:?}",
        result.err()
    );

    let query = result.unwrap();
    let having = query
        .select
        .having
        .as_ref()
        .expect("HAVING clause should exist");

    assert!(
        !having.operators.is_empty(),
        "HAVING AND operators should be captured"
    );
    assert_eq!(
        having.operators.len(),
        1,
        "Should have 1 operator for 2 conditions. Got: {:?}",
        having.operators
    );
    assert!(
        matches!(having.operators[0], LogicalOp::And),
        "Expected AND operator, got: {:?}",
        having.operators[0]
    );
}

// =============================================================================
// BUG 2+4: HAVING OR conditions silently treated as AND
// Parser captures conditions but ignores the logical operators
// =============================================================================

#[test]
fn test_bug_2_having_or_not_treated_as_and() {
    let sql = "SELECT category, COUNT(*) FROM products GROUP BY category HAVING COUNT(*) > 10 OR AVG(price) > 50";
    let result = Parser::parse(sql);

    assert!(result.is_ok());
    let query = result.unwrap();
    let having = query.select.having.as_ref().unwrap();

    // Should have 2 conditions
    assert_eq!(
        having.conditions.len(),
        2,
        "Should have 2 HAVING conditions"
    );

    // The operator between them should be OR, not AND
    if having.operators.is_empty() {
        panic!("Operators vec is empty - OR was not captured!");
    } else {
        assert!(
            matches!(having.operators[0], crate::velesql::LogicalOp::Or),
            "Operator should be OR, got: {:?}",
            having.operators[0]
        );
    }
}

// =============================================================================
// BUG 1: GROUP BY/HAVING unreachable from /query
// This is a DESIGN LIMITATION, not a bug:
// - execute_query() returns Vec<SearchResult> (for vector/text search)
// - execute_aggregate() returns serde_json::Value (for GROUP BY/HAVING)
// The server /query endpoint uses execute_query, so aggregations need a dedicated endpoint.
// TODO: Create /aggregate endpoint in velesdb-server (EPIC-042)
// =============================================================================

#[test]
fn test_bug_1_groupby_parses_correctly() {
    // GROUP BY is parsed correctly - the issue is execution dispatch, not parsing
    let sql = "SELECT category, COUNT(*) FROM products GROUP BY category HAVING COUNT(*) > 5";
    let result = Parser::parse(sql);

    assert!(
        result.is_ok(),
        "GROUP BY + HAVING should parse: {:?}",
        result.err()
    );

    let query = result.unwrap();
    assert!(
        query.select.group_by.is_some(),
        "GROUP BY should be present"
    );
    assert!(query.select.having.is_some(), "HAVING should be present");
}

// =============================================================================
// BUG 8: WITH(max_groups=...) is case-sensitive
// =============================================================================

#[test]
fn test_bug_8_with_max_groups_case_insensitive() {
    // Both should work identically
    let sql_lower = "SELECT category FROM products GROUP BY category WITH (max_groups = 100)";
    let sql_upper = "SELECT category FROM products GROUP BY category WITH (MAX_GROUPS = 100)";
    let sql_mixed = "SELECT category FROM products GROUP BY category WITH (Max_Groups = 100)";

    let result_lower = Parser::parse(sql_lower);
    let result_upper = Parser::parse(sql_upper);
    let result_mixed = Parser::parse(sql_mixed);

    assert!(result_lower.is_ok(), "lowercase max_groups should work");
    assert!(result_upper.is_ok(), "uppercase MAX_GROUPS should work");
    assert!(result_mixed.is_ok(), "mixed case Max_Groups should work");

    // All should produce the same parsed structure
    let query_lower = result_lower.unwrap();
    let query_upper = result_upper.unwrap();

    let with_lower = query_lower.select.with_clause.as_ref().unwrap();
    let with_upper = query_upper.select.with_clause.as_ref().unwrap();

    // Find max_groups option (case-insensitive comparison)
    let opt_lower = with_lower
        .options
        .iter()
        .find(|o| o.key.to_lowercase() == "max_groups");
    let opt_upper = with_upper
        .options
        .iter()
        .find(|o| o.key.to_lowercase() == "max_groups");

    assert!(
        opt_lower.is_some(),
        "max_groups option should be found in lowercase query"
    );
    assert!(
        opt_upper.is_some(),
        "MAX_GROUPS option should be found in uppercase query"
    );
}

#[test]
fn test_bug_5_correlated_field_dedup_in_subquery() {
    let sql = "SELECT * FROM products WHERE price > (SELECT AVG(price) FROM discounts WHERE `products.category` = 1 AND `products.category` = 2)";
    let result = Parser::parse(sql).expect("query should parse");

    let comparison = match result.select.where_clause.as_ref() {
        Some(crate::velesql::Condition::Comparison(comp)) => comp,
        other => panic!("expected WHERE comparison with subquery, got: {other:?}"),
    };

    let subquery = match &comparison.value {
        Value::Subquery(sub) => sub,
        other => panic!("expected subquery value, got: {other:?}"),
    };

    assert_eq!(
        subquery.correlations.len(),
        1,
        "duplicate outer references should be deduplicated"
    );
    assert_eq!(subquery.correlations[0].outer_table, "products");
    assert_eq!(subquery.correlations[0].outer_column, "category");
}

#[test]
fn test_bug_5_string_literals_not_treated_as_correlations() {
    let sql = "SELECT * FROM products WHERE price > (SELECT AVG(price) FROM discounts WHERE category = 'products.category')";
    let result = Parser::parse(sql).expect("query should parse");

    let comparison = match result.select.where_clause.as_ref() {
        Some(crate::velesql::Condition::Comparison(comp)) => comp,
        other => panic!("expected WHERE comparison with subquery, got: {other:?}"),
    };

    let subquery = match &comparison.value {
        Value::Subquery(sub) => sub,
        other => panic!("expected subquery value, got: {other:?}"),
    };

    assert!(
        subquery.correlations.is_empty(),
        "string literals must not create false correlations"
    );
}
