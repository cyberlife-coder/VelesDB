//! The order ORDER BY sorts JSON values in, shared by every engine and every
//! ORDER BY path: SELECT, grouped rows, vector GROUP BY, MATCH and window
//! functions, in core and in the WASM executor.

use std::cmp::Ordering;

use serde_json::Value;

/// Compares two JSON values in ascending ORDER BY order.
///
/// A missing value and `null` sort after every other value: `NULLS LAST` in
/// ascending order and, reversed, `NULLS FIRST` in descending order, as in
/// PostgreSQL. Other values sort by type first — booleans, numbers, strings,
/// arrays, objects — then within a type: `false` before `true`, numbers by
/// value, strings by code point. Two arrays, or two objects, compare equal.
#[must_use]
pub fn compare_json_values(a: Option<&Value>, b: Option<&Value>) -> Ordering {
    match (a, b) {
        (Some(Value::Number(x)), Some(Value::Number(y))) => x
            .as_f64()
            .unwrap_or(0.0)
            .total_cmp(&y.as_f64().unwrap_or(0.0)),
        (Some(Value::String(x)), Some(Value::String(y))) => x.cmp(y),
        (Some(Value::Bool(x)), Some(Value::Bool(y))) => x.cmp(y),
        _ => rank(a).cmp(&rank(b)),
    }
}

/// A value's place by type; missing and `null` last.
fn rank(value: Option<&Value>) -> u8 {
    match value {
        Some(Value::Bool(_)) => 0,
        Some(Value::Number(_)) => 1,
        Some(Value::String(_)) => 2,
        Some(Value::Array(_)) => 3,
        Some(Value::Object(_)) => 4,
        Some(Value::Null) | None => 5,
    }
}
