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
/// value (two integers exactly, beyond `f64` precision), strings by code point. Two arrays, or two objects, compare equal.
#[must_use]
pub fn compare_json_values(a: Option<&Value>, b: Option<&Value>) -> Ordering {
    match (a, b) {
        (Some(Value::Number(x)), Some(Value::Number(y))) => compare_numbers(x, y),
        (Some(Value::String(x)), Some(Value::String(y))) => x.cmp(y),
        (Some(Value::Bool(x)), Some(Value::Bool(y))) => x.cmp(y),
        _ => rank(a).cmp(&rank(b)),
    }
}

/// Numbers by value: two integers exactly, whatever their size, anything
/// else as `f64`.
fn compare_numbers(x: &serde_json::Number, y: &serde_json::Number) -> Ordering {
    match (integer(x), integer(y)) {
        (Some(x), Some(y)) => x.cmp(&y),
        _ => x
            .as_f64()
            .unwrap_or(0.0)
            .total_cmp(&y.as_f64().unwrap_or(0.0)),
    }
}

/// An integer number widened to `i128`, which holds every `i64` and `u64`.
fn integer(n: &serde_json::Number) -> Option<i128> {
    n.as_i64()
        .map(i128::from)
        .or_else(|| n.as_u64().map(i128::from))
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
