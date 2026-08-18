use super::*;
use serde_json::json;

#[test]
fn test_filter_eq() {
    let payload = json!({"category": "tech"});
    let filter = json!({
        "condition": {
            "type": "eq",
            "field": "category",
            "value": "tech"
        }
    });
    assert!(matches_filter(&payload, &filter));
}

#[test]
fn test_filter_neq() {
    let payload = json!({"category": "tech"});
    let filter = json!({
        "condition": {
            "type": "neq",
            "field": "category",
            "value": "sports"
        }
    });
    assert!(matches_filter(&payload, &filter));
}

#[test]
fn test_filter_gt() {
    let payload = json!({"score": 85.0});
    let filter = json!({
        "condition": {
            "type": "gt",
            "field": "score",
            "value": 80.0
        }
    });
    assert!(matches_filter(&payload, &filter));
}

#[test]
fn test_filter_and() {
    let payload = json!({"category": "tech", "score": 90.0});
    let filter = json!({
        "condition": {
            "type": "and",
            "conditions": [
                {"type": "eq", "field": "category", "value": "tech"},
                {"type": "gt", "field": "score", "value": 80.0}
            ]
        }
    });
    assert!(matches_filter(&payload, &filter));
}

#[test]
fn test_filter_or() {
    let payload = json!({"category": "sports"});
    let filter = json!({
        "condition": {
            "type": "or",
            "conditions": [
                {"type": "eq", "field": "category", "value": "tech"},
                {"type": "eq", "field": "category", "value": "sports"}
            ]
        }
    });
    assert!(matches_filter(&payload, &filter));
}

#[test]
fn test_filter_not() {
    let payload = json!({"category": "tech"});
    let filter = json!({
        "condition": {
            "type": "not",
            "condition": {
                "type": "eq",
                "field": "category",
                "value": "sports"
            }
        }
    });
    assert!(matches_filter(&payload, &filter));
}

#[test]
fn test_nested_field() {
    let payload = json!({"user": {"profile": {"name": "John"}}});
    let value = get_nested_field(&payload, "user.profile.name");
    assert_eq!(value, Some(&json!("John")));
}

#[test]
fn test_no_filter_matches_all() {
    let payload = json!({"anything": "value"});
    let filter = json!({});
    assert!(matches_filter(&payload, &filter));
}

#[test]
fn test_unknown_condition_type_fails_closed() {
    let payload = json!({"category": "tech"});
    let filter = json!({
        "condition": {
            "type": "eqals",
            "field": "category",
            "value": "tech"
        }
    });
    assert!(!matches_filter(&payload, &filter));
}
