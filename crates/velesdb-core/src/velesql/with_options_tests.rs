//! Tests for WITH clause options (EPIC-040 US-004).
//!
//! Covers:
//! - WITH(max_groups=N) for GROUP BY limit
//! - Parsing and execution of max_groups option

use crate::velesql::ast::WithValue;
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

    assert_eq!(
        max_groups.value,
        WithValue::Integer(100),
        "max_groups value must parse as Integer(100)"
    );
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
    assert_eq!(with_clause.options[0].key, "max_groups");
    assert_eq!(with_clause.options[0].value, WithValue::Integer(500));
    assert_eq!(with_clause.options[1].key, "timeout_ms");
    assert_eq!(with_clause.options[1].value, WithValue::Integer(1000));
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

    assert_eq!(
        group_limit.value,
        WithValue::Integer(50),
        "group_limit value must be parsed as Integer(50)"
    );
}
