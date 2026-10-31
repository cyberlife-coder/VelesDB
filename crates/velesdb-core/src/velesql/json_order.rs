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
/// exact value (an integer beyond `f64` precision included, `-0.0` equal to
/// `0`), strings by code point. Two arrays, or two objects, compare equal.
/// The order is total, so any sort may use it.
#[must_use]
pub fn compare_json_values(a: Option<&Value>, b: Option<&Value>) -> Ordering {
    match (a, b) {
        (Some(Value::Number(x)), Some(Value::Number(y))) => compare_numbers(x, y),
        (Some(Value::String(x)), Some(Value::String(y))) => x.cmp(y),
        (Some(Value::Bool(x)), Some(Value::Bool(y))) => x.cmp(y),
        _ => rank(a).cmp(&rank(b)),
    }
}

/// Numbers by exact value: two integers as integers, an integer and a float
/// exactly, two floats as `f64`. Comparing an integer with a float as `f64`
/// would round the integer and break transitivity: `2^53 + 1` would equal
/// `2^53` as a float, which is less than it as an integer.
pub(crate) fn compare_numbers(x: &serde_json::Number, y: &serde_json::Number) -> Ordering {
    match (integer(x), integer(y)) {
        (Some(x), Some(y)) => x.cmp(&y),
        (Some(x), None) => compare_integer_float(x, float(y)),
        (None, Some(y)) => compare_integer_float(y, float(x)).reverse(),
        (None, None) => compare_floats(float(x), float(y)),
    }
}

/// An integer and a float, exactly. Rounding to `f64` is monotonic, so a
/// strict order between the rounded integer and the float is their exact
/// order; on a tie the float is integral and within `i128`, and the two
/// integers decide.
fn compare_integer_float(integer: i128, float: f64) -> Ordering {
    // Reason: rounding is the point; a tie is settled exactly below.
    #[allow(clippy::cast_precision_loss)]
    let rounded = integer as f64;
    compare_floats(rounded, float).then_with(|| {
        // Reason: `float` equals a rounded i128, so it is integral and fits.
        #[allow(clippy::cast_possible_truncation)]
        let exact = float as i128;
        integer.cmp(&exact)
    })
}

/// Two floats, `-0.0` equal to `0.0`. A JSON number is never NaN.
fn compare_floats(x: f64, y: f64) -> Ordering {
    x.partial_cmp(&y).unwrap_or_else(|| x.total_cmp(&y))
}

/// A float number's value.
fn float(n: &serde_json::Number) -> f64 {
    n.as_f64().unwrap_or(0.0)
}

/// An integer number widened to `i128`, which holds every `i64` and `u64`.
pub(super) fn integer(n: &serde_json::Number) -> Option<i128> {
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
