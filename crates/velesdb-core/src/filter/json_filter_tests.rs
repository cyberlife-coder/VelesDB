//! Tests for JSON-to-Condition conversion.

use super::json_filter::json_to_condition;
use super::Filter;
use serde_json::json;

#[test]
fn test_eq_condition() {
    let filter = json!({"field": "name", "op": "eq", "value": "Alice"});
    let condition = json_to_condition(&filter).unwrap();

    let payload = json!({"name": "Alice"});
    assert!(condition.matches(&payload));

    let payload2 = json!({"name": "Bob"});
    assert!(!condition.matches(&payload2));
}

#[test]
fn test_neq_condition() {
    let filter = json!({"field": "status", "op": "neq", "value": "inactive"});
    let condition = json_to_condition(&filter).unwrap();

    let payload = json!({"status": "active"});
    assert!(condition.matches(&payload));

    let payload2 = json!({"status": "inactive"});
    assert!(!condition.matches(&payload2));
}

#[test]
fn test_gt_condition() {
    let filter = json!({"field": "age", "op": "gt", "value": 18});
    let condition = json_to_condition(&filter).unwrap();

    assert!(condition.matches(&json!({"age": 25})));
    assert!(!condition.matches(&json!({"age": 18})));
    assert!(!condition.matches(&json!({"age": 10})));
}

#[test]
fn test_gte_condition() {
    let filter = json!({"field": "score", "op": "gte", "value": 90});
    let condition = json_to_condition(&filter).unwrap();

    assert!(condition.matches(&json!({"score": 90})));
    assert!(condition.matches(&json!({"score": 95})));
    assert!(!condition.matches(&json!({"score": 89})));
}

#[test]
fn test_lt_condition() {
    let filter = json!({"field": "price", "op": "lt", "value": 100});
    let condition = json_to_condition(&filter).unwrap();

    assert!(condition.matches(&json!({"price": 50})));
    assert!(!condition.matches(&json!({"price": 100})));
}

#[test]
fn test_lte_condition() {
    let filter = json!({"field": "count", "op": "lte", "value": 5});
    let condition = json_to_condition(&filter).unwrap();

    assert!(condition.matches(&json!({"count": 5})));
    assert!(condition.matches(&json!({"count": 3})));
    assert!(!condition.matches(&json!({"count": 6})));
}

#[test]
fn test_in_condition() {
    let filter = json!({"field": "color", "op": "in", "values": ["red", "blue", "green"]});
    let condition = json_to_condition(&filter).unwrap();

    assert!(condition.matches(&json!({"color": "red"})));
    assert!(condition.matches(&json!({"color": "blue"})));
    assert!(!condition.matches(&json!({"color": "yellow"})));
}

#[test]
fn test_contains_condition() {
    let filter = json!({"field": "title", "op": "contains", "value": "Rust"});
    let condition = json_to_condition(&filter).unwrap();

    // Contains is case-sensitive
    assert!(condition.matches(&json!({"title": "Learning Rust"})));
    assert!(!condition.matches(&json!({"title": "Learning Python"})));
    assert!(!condition.matches(&json!({"title": "learning rust"}))); // case mismatch
}

#[test]
fn test_is_null_condition() {
    let filter = json!({"field": "email", "op": "is_null"});
    let condition = json_to_condition(&filter).unwrap();

    assert!(condition.matches(&json!({"name": "Alice"})));
    assert!(condition.matches(&json!({"email": null})));
    assert!(!condition.matches(&json!({"email": "a@b.com"})));
}

#[test]
fn test_is_not_null_condition() {
    let filter = json!({"field": "email", "op": "is_not_null"});
    let condition = json_to_condition(&filter).unwrap();

    assert!(condition.matches(&json!({"email": "a@b.com"})));
    assert!(!condition.matches(&json!({"name": "Alice"})));
}

#[test]
fn test_and_condition() {
    let filter = json!({
        "op": "and",
        "conditions": [
            {"field": "age", "op": "gte", "value": 18},
            {"field": "active", "op": "eq", "value": true}
        ]
    });
    let condition = json_to_condition(&filter).unwrap();

    assert!(condition.matches(&json!({"age": 25, "active": true})));
    assert!(!condition.matches(&json!({"age": 25, "active": false})));
    assert!(!condition.matches(&json!({"age": 10, "active": true})));
}

#[test]
fn test_or_condition() {
    let filter = json!({
        "op": "or",
        "conditions": [
            {"field": "role", "op": "eq", "value": "admin"},
            {"field": "role", "op": "eq", "value": "superadmin"}
        ]
    });
    let condition = json_to_condition(&filter).unwrap();

    assert!(condition.matches(&json!({"role": "admin"})));
    assert!(condition.matches(&json!({"role": "superadmin"})));
    assert!(!condition.matches(&json!({"role": "user"})));
}

#[test]
fn test_not_condition() {
    let filter = json!({
        "op": "not",
        "condition": {"field": "banned", "op": "eq", "value": true}
    });
    let condition = json_to_condition(&filter).unwrap();

    assert!(condition.matches(&json!({"banned": false})));
    assert!(!condition.matches(&json!({"banned": true})));
}

#[test]
fn test_nested_conditions() {
    let filter = json!({
        "op": "and",
        "conditions": [
            {"field": "age", "op": "gte", "value": 18},
            {
                "op": "or",
                "conditions": [
                    {"field": "country", "op": "eq", "value": "FR"},
                    {"field": "country", "op": "eq", "value": "US"}
                ]
            }
        ]
    });
    let condition = json_to_condition(&filter).unwrap();

    assert!(condition.matches(&json!({"age": 25, "country": "FR"})));
    assert!(condition.matches(&json!({"age": 30, "country": "US"})));
    assert!(!condition.matches(&json!({"age": 10, "country": "FR"})));
    assert!(!condition.matches(&json!({"age": 25, "country": "DE"})));
}

#[test]
fn test_filter_integration() {
    let filter_json = json!({"field": "category", "op": "eq", "value": "tech"});
    let condition = json_to_condition(&filter_json).unwrap();
    let filter = Filter::new(condition);

    assert!(filter.matches(&json!({"category": "tech"})));
    assert!(!filter.matches(&json!({"category": "sport"})));
}

#[test]
fn test_invalid_op_returns_none() {
    let filter = json!({"field": "x", "op": "invalid_op", "value": 1});
    assert!(json_to_condition(&filter).is_none());
}

#[test]
fn test_missing_field_returns_none() {
    let filter = json!({"op": "eq", "value": 1});
    assert!(json_to_condition(&filter).is_none());
}

#[test]
fn test_missing_op_returns_none() {
    let filter = json!({"field": "x", "value": 1});
    assert!(json_to_condition(&filter).is_none());
}

#[test]
fn test_non_object_returns_none() {
    assert!(json_to_condition(&json!("string")).is_none());
    assert!(json_to_condition(&json!(42)).is_none());
    assert!(json_to_condition(&json!(null)).is_none());
}

#[test]
fn test_like_condition() {
    let filter = json!({"field": "name", "op": "like", "pattern": "Al%"});
    let condition = json_to_condition(&filter).unwrap();

    assert!(condition.matches(&json!({"name": "Alice"})));
    assert!(!condition.matches(&json!({"name": "Bob"})));
}

#[test]
fn test_ilike_condition() {
    let filter = json!({"field": "name", "op": "ilike", "pattern": "al%"});
    let condition = json_to_condition(&filter).unwrap();

    assert!(condition.matches(&json!({"name": "Alice"})));
    assert!(condition.matches(&json!({"name": "ALFRED"})));
    assert!(!condition.matches(&json!({"name": "Bob"})));
}
