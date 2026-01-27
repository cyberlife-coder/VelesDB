//! Tests for `match_exec` module - MATCH clause execution.

use super::match_exec::*;

#[test]
fn test_match_result_creation() {
    let result = MatchResult::new(42, 2, vec![1, 2]);
    assert_eq!(result.node_id, 42);
    assert_eq!(result.depth, 2);
    assert_eq!(result.path, vec![1, 2]);
}

#[test]
fn test_match_result_with_binding() {
    let result = MatchResult::new(42, 0, vec![]).with_binding("n".to_string(), 42);
    assert_eq!(result.bindings.get("n"), Some(&42));
}
