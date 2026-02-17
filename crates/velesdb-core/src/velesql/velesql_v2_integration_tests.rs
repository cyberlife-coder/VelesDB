//! Integration tests for `VelesQL` v2.0 features (EPIC-040).
//!
//! Tests for:
//! - US-001: HAVING with AND/OR operators
//! - US-002: ORDER BY multi-expressions
//! - US-003: Extended JOIN (LEFT/RIGHT/FULL, USING)
//! - US-004: WITH `max_groups/group_limit`
//! - US-006: UNION/INTERSECT/EXCEPT set operations
//!
//! Note: US-005 (USING FUSION) tests are in `fusion_clause_tests.rs`

use crate::velesql::{Parser, SetOperator};

// =============================================================================
// US-001: HAVING with AND/OR operators
// =============================================================================

#[test]
fn test_having_with_and_operator() {
    let sql = "SELECT category, COUNT(*) FROM products GROUP BY category HAVING COUNT(*) > 5 AND SUM(price) > 1000";
    let result = Parser::parse(sql);
    assert!(
        result.is_ok(),
        "Should parse HAVING with AND: {:?}",
        result.err()
    );

    let query = result.unwrap();
    assert!(query.select.having.is_some());
    let having = query.select.having.unwrap();
    assert!(!having.conditions.is_empty());
}

#[test]
fn test_having_with_or_operator() {
    let sql = "SELECT category, COUNT(*) FROM products GROUP BY category HAVING COUNT(*) > 10 OR AVG(price) > 50";
    let result = Parser::parse(sql);
    assert!(
        result.is_ok(),
        "Should parse HAVING with OR: {:?}",
        result.err()
    );
}

#[test]
fn test_having_multiple_conditions() {
    // Current grammar: having_term AND/OR having_term (no parentheses)
    let sql = "SELECT category, COUNT(*) FROM products GROUP BY category HAVING COUNT(*) > 5 AND AVG(price) > 20 OR SUM(quantity) > 100";
    let result = Parser::parse(sql);
    assert!(
        result.is_ok(),
        "Should parse HAVING with multiple conditions: {:?}",
        result.err()
    );
}

// =============================================================================
// US-002: ORDER BY multi-expressions
// =============================================================================

#[test]
fn test_orderby_single_column() {
    let sql = "SELECT * FROM products ORDER BY price";
    let result = Parser::parse(sql);
    assert!(
        result.is_ok(),
        "Should parse single ORDER BY: {:?}",
        result.err()
    );

    let query = result.unwrap();
    assert!(query.select.order_by.is_some());
}

#[test]
fn test_orderby_multiple_columns() {
    let sql = "SELECT * FROM products ORDER BY category, price DESC, name ASC";
    let result = Parser::parse(sql);
    assert!(
        result.is_ok(),
        "Should parse multi-column ORDER BY: {:?}",
        result.err()
    );
}

// Note: ORDER BY aggregate requires grammar extension (future work)
// Current grammar supports ORDER BY column and ORDER BY similarity()

#[test]
fn test_orderby_with_similarity() {
    let sql = "SELECT * FROM documents ORDER BY similarity(vector, $query) DESC";
    let result = Parser::parse(sql);
    assert!(
        result.is_ok(),
        "Should parse ORDER BY similarity: {:?}",
        result.err()
    );
}

// =============================================================================
// US-003: JOIN clause (current implementation)
// Note: LEFT/RIGHT/FULL/USING planned for future
// =============================================================================

#[test]
fn test_inner_join_basic() {
    let sql = "SELECT * FROM orders JOIN customers ON orders.customer_id = customers.id";
    let result = Parser::parse(sql);
    assert!(
        result.is_ok(),
        "Should parse INNER JOIN: {:?}",
        result.err()
    );

    let query = result.unwrap();
    assert!(!query.select.joins.is_empty());
}

#[test]
fn test_join_with_alias() {
    let sql = "SELECT * FROM orders JOIN customers AS c ON orders.customer_id = c.id";
    let result = Parser::parse(sql);
    assert!(
        result.is_ok(),
        "Should parse JOIN with alias: {:?}",
        result.err()
    );
}

#[test]
fn test_multiple_joins() {
    let sql = "SELECT * FROM orders JOIN customers ON orders.customer_id = customers.id JOIN products ON orders.product_id = products.id";
    let result = Parser::parse(sql);
    assert!(
        result.is_ok(),
        "Should parse multiple JOINs: {:?}",
        result.err()
    );
}

// =============================================================================
// US-004: WITH max_groups/group_limit
// =============================================================================

#[test]
fn test_with_max_groups() {
    let sql = "SELECT category, COUNT(*) FROM products GROUP BY category WITH (max_groups = 100)";
    let result = Parser::parse(sql);
    assert!(
        result.is_ok(),
        "Should parse WITH max_groups: {:?}",
        result.err()
    );

    let query = result.unwrap();
    assert!(query.select.with_clause.is_some());
}

#[test]
fn test_with_group_limit() {
    let sql = "SELECT category, COUNT(*) FROM products GROUP BY category WITH (group_limit = 50)";
    let result = Parser::parse(sql);
    assert!(
        result.is_ok(),
        "Should parse WITH group_limit: {:?}",
        result.err()
    );
}

#[test]
fn test_with_multiple_options() {
    let sql = "SELECT * FROM docs WHERE vector NEAR $v LIMIT 10 WITH (mode = 'accurate', ef_search = 256, max_groups = 100)";
    let result = Parser::parse(sql);
    assert!(
        result.is_ok(),
        "Should parse WITH multiple options: {:?}",
        result.err()
    );
}

// =============================================================================
// US-005: USING FUSION hybrid search
// Note: Full tests in fusion_clause_tests.rs (feature branch)
// =============================================================================

// US-005 tests are on feature/EPIC-040-US-005-using-fusion branch
// They test: USING FUSION, USING FUSION(strategy='rrf', k=100), etc.

// =============================================================================
// US-006: UNION/INTERSECT/EXCEPT set operations
// =============================================================================

#[test]
fn test_union_basic() {
    let sql = "SELECT * FROM active_users UNION SELECT * FROM pending_users";
    let result = Parser::parse(sql);
    assert!(result.is_ok(), "Should parse UNION: {:?}", result.err());

    let query = result.unwrap();
    assert!(query.compound.is_some());
    let compound = query.compound.unwrap();
    assert_eq!(compound.operator, SetOperator::Union);
}

#[test]
fn test_union_all() {
    let sql = "SELECT * FROM orders_2024 UNION ALL SELECT * FROM orders_2025";
    let result = Parser::parse(sql);
    assert!(result.is_ok(), "Should parse UNION ALL: {:?}", result.err());

    let query = result.unwrap();
    let compound = query.compound.unwrap();
    assert_eq!(compound.operator, SetOperator::UnionAll);
}

#[test]
fn test_intersect() {
    let sql = "SELECT id FROM premium_users INTERSECT SELECT id FROM active_users";
    let result = Parser::parse(sql);
    assert!(result.is_ok(), "Should parse INTERSECT: {:?}", result.err());

    let query = result.unwrap();
    let compound = query.compound.unwrap();
    assert_eq!(compound.operator, SetOperator::Intersect);
}

#[test]
fn test_except() {
    let sql = "SELECT id FROM all_users EXCEPT SELECT id FROM banned_users";
    let result = Parser::parse(sql);
    assert!(result.is_ok(), "Should parse EXCEPT: {:?}", result.err());

    let query = result.unwrap();
    let compound = query.compound.unwrap();
    assert_eq!(compound.operator, SetOperator::Except);
}

#[test]
fn test_union_with_where_clauses() {
    let sql = "SELECT * FROM products WHERE category = 'electronics' UNION SELECT * FROM products WHERE price > 1000";
    let result = Parser::parse(sql);
    assert!(
        result.is_ok(),
        "Should parse UNION with WHERE: {:?}",
        result.err()
    );
}

#[test]
fn test_union_with_limit() {
    let sql = "SELECT * FROM recent_posts LIMIT 10 UNION SELECT * FROM popular_posts LIMIT 10";
    let result = Parser::parse(sql);
    assert!(
        result.is_ok(),
        "Should parse UNION with LIMIT: {:?}",
        result.err()
    );
}

// =============================================================================
// Combined features - Real-world scenarios
// =============================================================================

#[test]
fn test_analytics_query_with_groupby() {
    let sql = "SELECT category, COUNT(*), AVG(price) FROM products WHERE status = 'active' GROUP BY category HAVING COUNT(*) > 5 LIMIT 10 WITH (max_groups = 100)";
    let result = Parser::parse(sql);
    assert!(
        result.is_ok(),
        "Should parse analytics query: {:?}",
        result.err()
    );
}

#[test]
fn test_join_with_where() {
    // Note: WHERE condition uses simple identifier, not table.column
    let sql = "SELECT * FROM customers JOIN orders ON customers.id = orders.customer_id WHERE total > 100";
    let result = Parser::parse(sql);
    assert!(
        result.is_ok(),
        "Should parse JOIN with WHERE: {:?}",
        result.err()
    );
}

#[test]
fn test_set_operation_with_columns() {
    let sql = "SELECT name, score FROM active_users UNION SELECT name, score FROM archived_users";
    let result = Parser::parse(sql);
    assert!(
        result.is_ok(),
        "Should parse set operation: {:?}",
        result.err()
    );
}

// =============================================================================
// Edge cases and error handling
// =============================================================================

#[test]
fn test_count_without_group_by() {
    let sql = "SELECT COUNT(*) FROM products";
    let result = Parser::parse(sql);
    assert!(
        result.is_ok(),
        "Should parse COUNT without GROUP BY: {:?}",
        result.err()
    );
}

#[test]
fn test_multiple_aggregates_same_column() {
    let sql = "SELECT category, MIN(price), MAX(price), AVG(price) FROM products GROUP BY category";
    let result = Parser::parse(sql);
    assert!(
        result.is_ok(),
        "Should parse multiple aggregates: {:?}",
        result.err()
    );
}

#[test]
fn test_group_by_single_column() {
    let sql = "SELECT category, COUNT(*) FROM products GROUP BY category";
    let result = Parser::parse(sql);
    assert!(result.is_ok(), "Should parse GROUP BY: {:?}", result.err());
}
