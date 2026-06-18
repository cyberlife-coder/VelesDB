//! Tests for Self-JOIN support (EPIC-052 US-003).
//!
//! Covers:
//! - FROM table with alias
//! - Self-JOIN (same table with different aliases)
//! - Hierarchical queries (employee-manager pattern)

use crate::velesql::{JoinType, Parser};

// =============================================================================
// Parser Tests - FROM alias
// =============================================================================

#[test]
fn test_parse_from_with_alias() {
    let sql = "SELECT * FROM employees AS e";
    let result = Parser::parse(sql);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let query = result.unwrap();
    assert_eq!(query.select.from, "employees");
    assert!(query.select.from_alias.contains(&"e".to_string()));
}

#[test]
fn test_parse_from_without_alias() {
    let sql = "SELECT * FROM employees";
    let result = Parser::parse(sql);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let query = result.unwrap();
    assert_eq!(query.select.from, "employees");
    assert!(query.select.from_alias.is_empty());
}

// =============================================================================
// Parser Tests - Self-JOIN
// =============================================================================

#[test]
fn test_parse_self_join_basic() {
    let sql =
        "SELECT e.name, m.name FROM employees AS e JOIN employees AS m ON e.manager_id = m.id";
    let result = Parser::parse(sql);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let query = result.unwrap();
    assert_eq!(query.select.from, "employees");
    assert!(query.select.from_alias.contains(&"e".to_string()));
    assert_eq!(query.select.joins.len(), 1);
    assert_eq!(query.select.joins[0].table, "employees");
    assert_eq!(query.select.joins[0].alias, Some("m".to_string()));
}

#[test]
fn test_parse_self_join_left() {
    let sql =
        "SELECT e.name, m.name FROM employees AS e LEFT JOIN employees AS m ON e.manager_id = m.id";
    let result = Parser::parse(sql);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let query = result.unwrap();
    assert_eq!(query.select.joins[0].join_type, JoinType::Left);
}

#[test]
fn test_parse_self_join_with_where() {
    // Note: Qualified column names in WHERE (e.department) require separate enhancement
    // For now, test with simple column name
    let sql = "SELECT e.name, m.name FROM employees AS e JOIN employees AS m ON e.manager_id = m.id WHERE department = 'Engineering'";
    let result = Parser::parse(sql);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let query = result.unwrap();
    assert!(query.select.where_clause.is_some());
}

#[test]
fn test_parse_multiple_self_joins() {
    // Employee -> Manager -> Director hierarchy
    let sql = "SELECT e.name, m.name, d.name FROM employees AS e JOIN employees AS m ON e.manager_id = m.id JOIN employees AS d ON m.manager_id = d.id";
    let result = Parser::parse(sql);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let query = result.unwrap();
    assert!(query.select.from_alias.contains(&"e".to_string()));
    assert_eq!(query.select.joins.len(), 2);
    assert_eq!(query.select.joins[0].alias, Some("m".to_string()));
    assert_eq!(query.select.joins[1].alias, Some("d".to_string()));
}

// =============================================================================
// BUG-8: Multi-alias FROM — from_alias Vec contains all aliases in scope
// =============================================================================

#[test]
fn test_bug8_self_join_from_alias_contains_both_aliases() {
    // BUG-8: Previously from_alias was Option<String> holding only the FROM alias.
    // Now it is Vec<String> holding FROM alias + JOIN aliases.
    let sql =
        "SELECT e.name, m.name FROM employees AS e JOIN employees AS m ON e.manager_id = m.id";
    let query = Parser::parse(sql).unwrap();

    // from_alias should contain both "e" (FROM) and "m" (JOIN)
    assert_eq!(query.select.from_alias.len(), 2);
    assert_eq!(query.select.from_alias[0], "e");
    assert_eq!(query.select.from_alias[1], "m");
}

#[test]
fn test_bug8_triple_join_from_alias_contains_all_aliases() {
    let sql = "SELECT e.name, m.name, d.name FROM employees AS e JOIN employees AS m ON e.manager_id = m.id JOIN employees AS d ON m.manager_id = d.id";
    let query = Parser::parse(sql).unwrap();

    // from_alias should contain "e" (FROM), "m" (JOIN1), "d" (JOIN2)
    assert_eq!(query.select.from_alias.len(), 3);
    assert_eq!(query.select.from_alias[0], "e");
    assert_eq!(query.select.from_alias[1], "m");
    assert_eq!(query.select.from_alias[2], "d");
}

#[test]
fn test_bug8_no_alias_produces_empty_vec() {
    let sql = "SELECT * FROM employees";
    let query = Parser::parse(sql).unwrap();
    assert!(query.select.from_alias.is_empty());
}

#[test]
fn test_bug8_single_from_alias_no_join() {
    let sql = "SELECT * FROM docs AS d";
    let query = Parser::parse(sql).unwrap();
    assert_eq!(query.select.from_alias, vec!["d".to_string()]);
}

#[test]
fn test_bug8_join_without_from_alias() {
    // FROM has no alias but JOIN does — only JOIN alias appears
    let sql = "SELECT * FROM docs JOIN tags AS t ON docs.id = t.doc_id";
    let query = Parser::parse(sql).unwrap();
    assert_eq!(query.select.from_alias, vec!["t".to_string()]);
}

// =============================================================================
// Edge Cases
// =============================================================================

#[test]
fn test_parse_from_alias_case_insensitive() {
    let sql = "SELECT * FROM employees AS E";
    let result = Parser::parse(sql);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let query = result.unwrap();
    assert!(query.select.from_alias.contains(&"E".to_string()));
}

#[test]
fn test_parse_self_join_preserves_table_name() {
    // Both FROM and JOIN reference same table
    let sql = "SELECT * FROM products AS p JOIN products AS related ON p.related_id = related.id";
    let result = Parser::parse(sql);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let query = result.unwrap();
    assert_eq!(query.select.from, "products");
    assert_eq!(query.select.joins[0].table, "products");
}
