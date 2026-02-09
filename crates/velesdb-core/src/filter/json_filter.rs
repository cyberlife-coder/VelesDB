//! JSON-to-Condition conversion for WASM and external consumers.
//!
//! Converts a `serde_json::Value` filter object (the format WASM uses)
//! to core's `Condition` type, enabling WASM to delegate filter evaluation
//! to core instead of reimplementing matching logic.
//!
//! # JSON Format
//!
//! ```json
//! { "field": "name", "op": "eq", "value": "hello" }
//! { "op": "and", "conditions": [
//!     { "field": "age", "op": "gt", "value": 18 },
//!     { "field": "active", "op": "eq", "value": true }
//! ]}
//! ```

use super::Condition;
use serde_json::Value;

/// Converts a JSON filter object to a core `Condition`.
///
/// Supports operators: `eq`, `neq`, `gt`, `gte`, `lt`, `lte`,
/// `in`, `contains`, `is_null`, `is_not_null`, `like`, `ilike`,
/// `and`, `or`, `not`.
///
/// # Format
///
/// Comparison: `{ "field": "x", "op": "eq", "value": "y" }`
/// Logical AND/OR: `{ "op": "and", "conditions": [...] }`
/// Logical NOT: `{ "op": "not", "condition": { ... } }`
///
/// # Returns
///
/// `Some(Condition)` if the JSON is a valid filter, `None` otherwise.
#[must_use]
pub fn json_to_condition(filter: &Value) -> Option<Condition> {
    let obj = filter.as_object()?;
    let op = obj.get("op")?.as_str()?;

    match op {
        "eq" => {
            let field = obj.get("field")?.as_str()?.to_string();
            let value = obj.get("value")?.clone();
            Some(Condition::eq(field, value))
        }
        "neq" => {
            let field = obj.get("field")?.as_str()?.to_string();
            let value = obj.get("value")?.clone();
            Some(Condition::neq(field, value))
        }
        "gt" => {
            let field = obj.get("field")?.as_str()?.to_string();
            let value = obj.get("value")?.clone();
            Some(Condition::Gt { field, value })
        }
        "gte" => {
            let field = obj.get("field")?.as_str()?.to_string();
            let value = obj.get("value")?.clone();
            Some(Condition::Gte { field, value })
        }
        "lt" => {
            let field = obj.get("field")?.as_str()?.to_string();
            let value = obj.get("value")?.clone();
            Some(Condition::Lt { field, value })
        }
        "lte" => {
            let field = obj.get("field")?.as_str()?.to_string();
            let value = obj.get("value")?.clone();
            Some(Condition::Lte { field, value })
        }
        "in" => {
            let field = obj.get("field")?.as_str()?.to_string();
            let values = obj.get("values")?.as_array()?.clone();
            Some(Condition::In { field, values })
        }
        "contains" => {
            let field = obj.get("field")?.as_str()?.to_string();
            let value = obj.get("value")?.as_str()?.to_string();
            Some(Condition::contains(field, value))
        }
        "is_null" => {
            let field = obj.get("field")?.as_str()?.to_string();
            Some(Condition::is_null(field))
        }
        "is_not_null" => {
            let field = obj.get("field")?.as_str()?.to_string();
            Some(Condition::is_not_null(field))
        }
        "like" => {
            let field = obj.get("field")?.as_str()?.to_string();
            let pattern = obj.get("pattern")?.as_str()?.to_string();
            Some(Condition::like(field, pattern))
        }
        "ilike" => {
            let field = obj.get("field")?.as_str()?.to_string();
            let pattern = obj.get("pattern")?.as_str()?.to_string();
            Some(Condition::ilike(field, pattern))
        }
        "and" => {
            let conditions = obj.get("conditions")?.as_array()?;
            let parsed: Vec<Condition> = conditions.iter().filter_map(json_to_condition).collect();
            Some(Condition::and(parsed))
        }
        "or" => {
            let conditions = obj.get("conditions")?.as_array()?;
            let parsed: Vec<Condition> = conditions.iter().filter_map(json_to_condition).collect();
            Some(Condition::or(parsed))
        }
        "not" => {
            let inner = obj.get("condition")?;
            let parsed = json_to_condition(inner)?;
            Some(Condition::not(parsed))
        }
        _ => None,
    }
}
