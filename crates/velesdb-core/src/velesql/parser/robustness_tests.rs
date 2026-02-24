//! Robustness regression tests for parser panic-prone paths.

use crate::velesql::Parser;

#[test]
fn parse_join_condition_handles_quoted_identifiers_with_dots() {
    let query = r#"SELECT * FROM users AS u JOIN orders AS o ON `tenant.users`.id = "order.items"."user""id""#;

    let parsed = Parser::parse(query).expect("query with quoted JOIN identifiers should parse");
    let join = parsed
        .select
        .joins
        .first()
        .expect("expected one JOIN clause");
    let condition = join
        .condition
        .as_ref()
        .expect("expected JOIN condition in ON clause");

    assert_eq!(condition.left.table.as_deref(), Some("tenant.users"));
    assert_eq!(condition.left.column, "id");
    assert_eq!(condition.right.table.as_deref(), Some("order.items"));
    assert_eq!(condition.right.column, "user\"id");
}
