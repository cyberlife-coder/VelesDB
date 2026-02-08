//! Tests for WITH clause options (EPIC-040 US-004).
//!
//! Covers:
//! - WITH(max_groups=N) for GROUP BY limit
//! - Parsing and execution of max_groups option

use crate::velesql::Parser;

#[test]
fn test_with_max_groups_parsing() {
    let sql = "SELECT category, COUNT(*) FROM products GROUP BY category WITH (max_groups = 100)";
    let result = Parser::parse(sql);
    assert!(
        result.is_ok(),
        "Failed to parse WITH max_groups: {:?}",
        result.err()
    );

    let query = result.unwrap();
    let with_clause = query
        .select
        .with_clause
        .as_ref()
        .expect("WITH clause should be present");

    // Find max_groups option
    let max_groups = with_clause
        .options
        .iter()
        .find(|opt| opt.key == "max_groups")
        .expect("max_groups option should be present");

    assert_eq!(max_groups.key, "max_groups");
}

#[test]
fn test_with_multiple_options() {
    let sql = "SELECT * FROM docs WITH (max_groups = 500, timeout_ms = 1000)";
    let result = Parser::parse(sql);
    assert!(
        result.is_ok(),
        "Failed to parse WITH multiple options: {:?}",
        result.err()
    );

    let query = result.unwrap();
    let with_clause = query
        .select
        .with_clause
        .as_ref()
        .expect("WITH clause should be present");

    assert_eq!(with_clause.options.len(), 2);
}

#[test]
fn test_with_group_limit_option() {
    // Alternative name: group_limit instead of max_groups
    let sql = "SELECT category, COUNT(*) FROM products GROUP BY category WITH (group_limit = 50)";
    let result = Parser::parse(sql);
    assert!(
        result.is_ok(),
        "Failed to parse WITH group_limit: {:?}",
        result.err()
    );

    let query = result.unwrap();
    let with_clause = query
        .select
        .with_clause
        .as_ref()
        .expect("WITH clause should be present");

    let group_limit = with_clause
        .options
        .iter()
        .find(|opt| opt.key == "group_limit")
        .expect("group_limit option should be present");

    assert_eq!(group_limit.key, "group_limit");
}

// =========================================================================
// D-04: Overfetch factor tests
// =========================================================================

#[test]
fn test_with_overfetch_default_none() {
    use crate::velesql::ast::WithClause;
    let clause = WithClause::new();
    assert_eq!(clause.get_overfetch(), None);
}

#[test]
fn test_with_overfetch_custom_value() {
    use crate::velesql::ast::{WithClause, WithValue};
    let clause = WithClause::new().with_option("overfetch", WithValue::Integer(20));
    assert_eq!(clause.get_overfetch(), Some(20));
}

#[test]
fn test_with_overfetch_clamped_min() {
    use crate::velesql::ast::{WithClause, WithValue};
    let clause = WithClause::new().with_option("overfetch", WithValue::Integer(0));
    assert_eq!(clause.get_overfetch(), Some(1));

    let clause_neg = WithClause::new().with_option("overfetch", WithValue::Integer(-5));
    assert_eq!(clause_neg.get_overfetch(), Some(1));
}

#[test]
fn test_with_overfetch_clamped_max() {
    use crate::velesql::ast::{WithClause, WithValue};
    let clause = WithClause::new().with_option("overfetch", WithValue::Integer(200));
    assert_eq!(clause.get_overfetch(), Some(100));
}

#[test]
fn test_with_overfetch_parsing() {
    let sql = "SELECT * FROM docs WHERE vector NEAR $v LIMIT 10 WITH (overfetch = 25)";
    let result = Parser::parse(sql);
    assert!(
        result.is_ok(),
        "Failed to parse WITH overfetch: {:?}",
        result.err()
    );

    let query = result.unwrap();
    let with_clause = query
        .select
        .with_clause
        .as_ref()
        .expect("WITH clause should be present");

    assert_eq!(with_clause.get_overfetch(), Some(25));
}
