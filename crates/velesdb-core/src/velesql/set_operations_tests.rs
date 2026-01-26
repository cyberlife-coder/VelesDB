//! Tests for SQL set operations (EPIC-040 US-006).
//!
//! Covers:
//! - UNION / UNION ALL
//! - INTERSECT
//! - EXCEPT
//! - Chained operations

use crate::velesql::Parser;

#[test]
fn test_union_basic() {
    let sql = "SELECT * FROM products WHERE category = 'electronics' UNION SELECT * FROM products WHERE price > 100";
    let result = Parser::parse(sql);
    assert!(result.is_ok(), "Failed to parse UNION: {:?}", result.err());

    let query = result.unwrap();
    let compound = query
        .compound
        .as_ref()
        .expect("Compound query should be present");

    assert_eq!(compound.operator, crate::velesql::SetOperator::Union);
}

#[test]
fn test_union_all() {
    let sql = "SELECT id FROM docs WHERE author = 'Alice' UNION ALL SELECT id FROM docs WHERE topic = 'AI'";
    let result = Parser::parse(sql);
    assert!(
        result.is_ok(),
        "Failed to parse UNION ALL: {:?}",
        result.err()
    );

    let query = result.unwrap();
    let compound = query
        .compound
        .as_ref()
        .expect("Compound query should be present");

    assert_eq!(compound.operator, crate::velesql::SetOperator::UnionAll);
}

#[test]
fn test_intersect() {
    let sql = "SELECT id FROM active_users INTERSECT SELECT id FROM premium_users";
    let result = Parser::parse(sql);
    assert!(
        result.is_ok(),
        "Failed to parse INTERSECT: {:?}",
        result.err()
    );

    let query = result.unwrap();
    let compound = query
        .compound
        .as_ref()
        .expect("Compound query should be present");

    assert_eq!(compound.operator, crate::velesql::SetOperator::Intersect);
}

#[test]
fn test_except() {
    let sql = "SELECT id FROM all_users EXCEPT SELECT id FROM banned_users";
    let result = Parser::parse(sql);
    assert!(result.is_ok(), "Failed to parse EXCEPT: {:?}", result.err());

    let query = result.unwrap();
    let compound = query
        .compound
        .as_ref()
        .expect("Compound query should be present");

    assert_eq!(compound.operator, crate::velesql::SetOperator::Except);
}

#[test]
fn test_simple_select_no_compound() {
    // Ensure simple SELECT still works and has no compound
    let sql = "SELECT * FROM docs";
    let result = Parser::parse(sql);
    assert!(result.is_ok());

    let query = result.unwrap();
    assert!(query.compound.is_none());
}
