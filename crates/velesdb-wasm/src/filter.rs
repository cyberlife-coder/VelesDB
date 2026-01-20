//! Filter evaluation for `VelesDB` WASM.
//!
//! Provides JSON-based metadata filtering for vector search results.
//! Supports conditions: eq, neq, gt, gte, lt, lte, and, or, not.

use serde_json::Value;

/// Evaluates if a payload matches a filter condition.
///
/// # Filter Format
///
/// ```json
/// {
///   "condition": {
///     "type": "eq",
///     "field": "category",
///     "value": "tech"
///   }
/// }
/// ```
///
/// # Supported Condition Types
///
/// - `eq`: Equals
/// - `neq`: Not equals
/// - `gt`, `gte`: Greater than (or equal)
/// - `lt`, `lte`: Less than (or equal)
/// - `and`: All conditions must match
/// - `or`: Any condition must match
/// - `not`: Negates inner condition
pub fn matches_filter(payload: &Value, filter: &Value) -> bool {
    let condition = match filter.get("condition") {
        Some(c) => c,
        None => return true,
    };

    evaluate_condition(payload, condition)
}

/// Evaluates a single condition against a payload.
pub fn evaluate_condition(payload: &Value, condition: &Value) -> bool {
    let cond_type = condition.get("type").and_then(|t| t.as_str()).unwrap_or("");

    match cond_type {
        "eq" => {
            let field = condition
                .get("field")
                .and_then(|f| f.as_str())
                .unwrap_or("");
            let value = condition.get("value");
            let payload_value = get_nested_field(payload, field);
            match (payload_value, value) {
                (Some(pv), Some(v)) => pv == v,
                _ => false,
            }
        }
        "neq" => {
            let field = condition
                .get("field")
                .and_then(|f| f.as_str())
                .unwrap_or("");
            let value = condition.get("value");
            let payload_value = get_nested_field(payload, field);
            match (payload_value, value) {
                (Some(pv), Some(v)) => pv != v,
                (None, _) => true,
                _ => false,
            }
        }
        "gt" => {
            let field = condition
                .get("field")
                .and_then(|f| f.as_str())
                .unwrap_or("");
            let value = condition.get("value").and_then(|v| v.as_f64());
            let payload_value = get_nested_field(payload, field).and_then(|v| v.as_f64());
            match (payload_value, value) {
                (Some(pv), Some(v)) => pv > v,
                _ => false,
            }
        }
        "gte" => {
            let field = condition
                .get("field")
                .and_then(|f| f.as_str())
                .unwrap_or("");
            let value = condition.get("value").and_then(|v| v.as_f64());
            let payload_value = get_nested_field(payload, field).and_then(|v| v.as_f64());
            match (payload_value, value) {
                (Some(pv), Some(v)) => pv >= v,
                _ => false,
            }
        }
        "lt" => {
            let field = condition
                .get("field")
                .and_then(|f| f.as_str())
                .unwrap_or("");
            let value = condition.get("value").and_then(|v| v.as_f64());
            let payload_value = get_nested_field(payload, field).and_then(|v| v.as_f64());
            match (payload_value, value) {
                (Some(pv), Some(v)) => pv < v,
                _ => false,
            }
        }
        "lte" => {
            let field = condition
                .get("field")
                .and_then(|f| f.as_str())
                .unwrap_or("");
            let value = condition.get("value").and_then(|v| v.as_f64());
            let payload_value = get_nested_field(payload, field).and_then(|v| v.as_f64());
            match (payload_value, value) {
                (Some(pv), Some(v)) => pv <= v,
                _ => false,
            }
        }
        "and" => {
            let conditions = condition.get("conditions").and_then(|c| c.as_array());
            match conditions {
                Some(conds) => conds.iter().all(|c| evaluate_condition(payload, c)),
                None => true,
            }
        }
        "or" => {
            let conditions = condition.get("conditions").and_then(|c| c.as_array());
            match conditions {
                Some(conds) => conds.iter().any(|c| evaluate_condition(payload, c)),
                None => true,
            }
        }
        "not" => {
            let inner = condition.get("condition");
            match inner {
                Some(c) => !evaluate_condition(payload, c),
                None => true,
            }
        }
        _ => true,
    }
}

/// Gets a nested field from a JSON payload using dot notation.
///
/// # Example
///
/// ```ignore
/// let payload = json!({"user": {"name": "John"}});
/// let name = get_nested_field(&payload, "user.name");
/// assert_eq!(name, Some(&json!("John")));
/// ```
pub fn get_nested_field<'a>(payload: &'a Value, field: &str) -> Option<&'a Value> {
    let mut current = payload;
    for part in field.split('.') {
        current = current.get(part)?;
    }
    Some(current)
}

#[cfg(test)]
mod tests {
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
}
