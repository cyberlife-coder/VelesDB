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
            let (pv, v) = match extract_field_pair(payload, condition) {
                Some(pair) => pair,
                None => return false,
            };
            pv == v
        }
        "neq" => match extract_field_pair(payload, condition) {
            Some((pv, v)) => pv != v,
            None => {
                // No condition value → neq always true (backward compat).
                // Condition value present but field missing → also true.
                condition.get("value").is_none()
                    || condition
                        .get("field")
                        .and_then(|f| f.as_str())
                        .is_some_and(|field| get_nested_field(payload, field).is_none())
            }
        },
        "gt" => compare_numeric(payload, condition, |pv, v| pv > v),
        "gte" => compare_numeric(payload, condition, |pv, v| pv >= v),
        "lt" => compare_numeric(payload, condition, |pv, v| pv < v),
        "lte" => compare_numeric(payload, condition, |pv, v| pv <= v),
        "and" => condition
            .get("conditions")
            .and_then(|c| c.as_array())
            .is_none_or(|conds| conds.iter().all(|c| evaluate_condition(payload, c))),
        "or" => condition
            .get("conditions")
            .and_then(|c| c.as_array())
            .is_none_or(|conds| conds.iter().any(|c| evaluate_condition(payload, c))),
        "not" => condition
            .get("condition")
            .is_none_or(|c| !evaluate_condition(payload, c)),
        _ => true,
    }
}

/// Extracts the (payload_value, condition_value) pair from a condition with "field" and "value" keys.
fn extract_field_pair<'a>(
    payload: &'a Value,
    condition: &'a Value,
) -> Option<(&'a Value, &'a Value)> {
    let field = condition
        .get("field")
        .and_then(|f| f.as_str())
        .unwrap_or("");
    let cond_value = condition.get("value")?;
    let payload_value = get_nested_field(payload, field)?;
    Some((payload_value, cond_value))
}

/// Evaluates a numeric comparison condition using the given comparator.
fn compare_numeric(payload: &Value, condition: &Value, cmp: fn(f64, f64) -> bool) -> bool {
    let field = condition
        .get("field")
        .and_then(|f| f.as_str())
        .unwrap_or("");
    let cond_val = condition.get("value").and_then(|v| v.as_f64());
    let payload_val = get_nested_field(payload, field).and_then(|v| v.as_f64());
    match (payload_val, cond_val) {
        (Some(pv), Some(v)) => cmp(pv, v),
        _ => false,
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
