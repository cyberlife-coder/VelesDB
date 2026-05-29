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

#[test]
fn parse_rejects_excessive_condition_nesting_depth() {
    // #896: deeply nested parentheses are now rejected by the raw-byte
    // pre-scan BEFORE pest can build the tree (and before the AST-level
    // MAX_CONDITION_DEPTH guard would have fired).
    let depth = 257;
    let mut query = String::from("SELECT * FROM t WHERE ");
    for _ in 0..depth {
        query.push_str("NOT (");
    }
    query.push_str("x = 1");
    for _ in 0..depth {
        query.push(')');
    }

    let err = Parser::parse(&query).expect_err("deeply nested condition should be rejected");
    assert!(
        err.to_string().contains("Query nesting too deep"),
        "unexpected parser error: {err}"
    );
}

/// #896 vector 1: ~3000 nested parens in WHERE `primary_expr` previously
/// overflowed pest's recursive descent (SIGABRT). Must now `Err` quickly.
#[test]
fn parse_rejects_deeply_nested_where_parens_without_overflow() {
    let depth = 5000;
    let query = format!(
        "SELECT * FROM t WHERE {}x = 1{}",
        "(".repeat(depth),
        ")".repeat(depth)
    );

    let err = Parser::parse(&query).expect_err("deeply nested WHERE parens must be rejected");
    assert!(
        err.to_string().contains("Query nesting too deep"),
        "unexpected parser error: {err}"
    );
}

/// #896 vector 2: ~3000 nested parens in ORDER BY `arithmetic_atom`.
#[test]
fn parse_rejects_deeply_nested_order_by_arithmetic_without_overflow() {
    let depth = 5000;
    let query = format!(
        "SELECT * FROM t ORDER BY {}1{}",
        "(".repeat(depth),
        ")".repeat(depth)
    );

    let err =
        Parser::parse(&query).expect_err("deeply nested ORDER BY arithmetic must be rejected");
    assert!(
        err.to_string().contains("Query nesting too deep"),
        "unexpected parser error: {err}"
    );
}

/// #896 vector 3: deeply nested `(SELECT … WHERE x = (SELECT …))` subqueries.
#[test]
fn parse_rejects_deeply_nested_subqueries_without_overflow() {
    // Depth kept under max_query_length (16384 bytes) so the nesting guard,
    // not the length guard, is the rejecting check. 200 levels still far
    // exceeds the 64-level nesting bound.
    let depth = 200;
    let mut query = String::from("SELECT * FROM t WHERE x = ");
    for _ in 0..depth {
        query.push_str("(SELECT id FROM t WHERE y = ");
    }
    query.push('1');
    query.push_str(&")".repeat(depth));

    let err = Parser::parse(&query).expect_err("deeply nested subqueries must be rejected");
    assert!(
        err.to_string().contains("Query nesting too deep"),
        "unexpected parser error: {err}"
    );
}

/// Parens inside a string literal must NOT count toward the nesting depth
/// (no false positive from the pre-scan).
#[test]
fn parse_does_not_count_parens_inside_string_literal() {
    let payload = "(".repeat(5000);
    let query = format!("SELECT * FROM t WHERE name = '{payload}'");

    // Must not be rejected for nesting; the literal parses fine.
    let parsed = Parser::parse(&query)
        .expect("parens inside a string literal must not trip the depth guard");
    assert!(parsed.select.where_clause.is_some());
}

/// A normal moderately-nested query still parses fine (no regression).
#[test]
fn parse_accepts_moderately_nested_query() {
    let query = "SELECT * FROM t WHERE ((a = 1 AND b = 2) OR (c = 3 AND (d = 4 OR e = 5)))";
    let parsed = Parser::parse(query).expect("moderately nested query should parse");
    assert!(parsed.select.where_clause.is_some());
}

/// #896 follow-up defect 1 (critical): a bracket-free `NOT NOT NOT …` chain
/// recurses through `not_expr = { ^"NOT" ~ primary_expr }` with NO delimiter,
/// so the bracket-only scan missed it and pest overflowed the stack (SIGABRT).
/// The prefix-run metric must now reject it quickly without parsing.
#[test]
fn parse_rejects_bracket_free_not_chain_without_overflow() {
    // 4000 `NOT ` = 16000 bytes, deliberately under the 16384 length cap so the
    // prefix-run depth metric (not the length guard) is the rejecting check.
    let query = format!("SELECT * FROM t WHERE {}a = 1", "NOT ".repeat(4000));

    let err = Parser::parse(&query).expect_err("bracket-free NOT chain must be rejected");
    assert!(
        err.to_string().contains("Query nesting too deep"),
        "unexpected parser error: {err}"
    );
}

/// #896 follow-up defect 2 (critical): an apostrophe inside a `--` comment must
/// NOT flip the scanner into a never-closing single-quote state. If it did, the
/// deep real parens after the comment would be treated as in-string and the
/// guard would be bypassed (SIGABRT). Comment-skipping must neutralise it.
#[test]
fn parse_rejects_deep_parens_after_comment_with_apostrophe() {
    let query = format!(
        "SELECT * FROM t -- it's deep\nWHERE {}a = 1{}",
        "(".repeat(5000),
        ")".repeat(5000)
    );

    let err =
        Parser::parse(&query).expect_err("deep parens after a poisoned comment must be rejected");
    assert!(
        err.to_string().contains("Query nesting too deep"),
        "unexpected parser error: {err}"
    );
}

/// #896 follow-up defect 3 (false positive): brackets/quotes inside a `--`
/// line comment must NOT be counted, so a query with a comment full of `(((`
/// and an apostrophe still parses normally.
#[test]
fn parse_does_not_count_brackets_inside_line_comment() {
    let comment = "(".repeat(5000);
    let query = format!("SELECT * FROM t -- {comment} it's fine\nWHERE a = 1");

    let parsed =
        Parser::parse(&query).expect("brackets inside a -- comment must not trip the depth guard");
    assert!(parsed.select.where_clause.is_some());
}

/// No regression: a small legitimate `NOT` nesting under the bound parses fine
/// and the prefix run does not accumulate falsely across operands.
#[test]
fn parse_accepts_small_not_nesting() {
    let query = "SELECT * FROM t WHERE NOT NOT a = 1 AND NOT b = 2";
    let parsed = Parser::parse(query).expect("small NOT nesting should parse");
    assert!(parsed.select.where_clause.is_some());
}
