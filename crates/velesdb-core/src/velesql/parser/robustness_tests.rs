//! Robustness regression tests for parser panic-prone paths.

use crate::velesql::Parser;

#[test]
fn parse_join_condition_does_not_panic_for_valid_join_query() {
    let query = "SELECT * FROM users AS u JOIN orders AS o ON u.id = o.user_id";

    let result = std::panic::catch_unwind(|| Parser::parse(query));

    assert!(result.is_ok(), "parser panicked on a valid JOIN query");
    match result {
        Ok(parse_result) => assert!(parse_result.is_ok()),
        Err(_) => unreachable!("assert above guarantees Ok"),
    }
}
