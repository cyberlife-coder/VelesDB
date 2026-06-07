//! Condition matching logic and helper functions.

use super::Condition;
use crate::metrics::global_guardrails_metrics;
use serde_json::Value;

const LIKE_MAX_PATTERN_BYTES: usize = 4096;
const LIKE_MAX_DYN_OPS: usize = 2_000_000;

/// List length above which IN matching switches from O(n) linear scan to
/// O(log n) binary search. Values must be pre-sorted by [`json_value_cmp`]
/// at `Condition::In` construction time (done in `conversion.rs::convert_in`).
const IN_BINARY_SEARCH_THRESHOLD: usize = 16;

/// Assigns a numeric rank to a JSON value type for consistent total ordering.
///
/// Type rank: Null(0) < Bool(1) < Number(2) < String(3) < Array(4) < Object(5).
fn json_type_rank(v: &Value) -> u8 {
    match v {
        Value::Null => 0,
        Value::Bool(_) => 1,
        Value::Number(_) => 2,
        Value::String(_) => 3,
        Value::Array(_) => 4,
        Value::Object(_) => 5,
    }
}

/// Total ordering over `serde_json::Value` used for sorting IN-list values
/// and binary-searching them at match time (issue #512).
///
/// Within each type: Null = Null, Bool by value, Number by `f64::total_cmp`
/// (NaN sorts after +∞, giving a deterministic total order), String
/// lexicographic, Arrays and Objects by type rank only (equal among themselves,
/// which is acceptable since they are not used in IN conditions in practice).
pub(super) fn json_value_cmp(a: &Value, b: &Value) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let rank = json_type_rank(a).cmp(&json_type_rank(b));
    if rank != Ordering::Equal {
        return rank;
    }
    match (a, b) {
        (Value::Bool(a), Value::Bool(b)) => a.cmp(b),
        (Value::Number(a), Value::Number(b)) => a
            .as_f64()
            .zip(b.as_f64())
            .map_or(Ordering::Equal, |(fa, fb)| fa.total_cmp(&fb)),
        (Value::String(a), Value::String(b)) => a.cmp(b),
        _ => Ordering::Equal,
    }
}

/// Returns whether `field_val` appears in `values`.
///
/// Switches strategy based on list length:
/// - `values.len() <= IN_BINARY_SEARCH_THRESHOLD`: O(n) linear scan via
///   `values_equal` (epsilon-tolerant float equality).
/// - `values.len() > IN_BINARY_SEARCH_THRESHOLD`: O(log n) lower-bound via
///   `json_value_cmp` to narrow position, then `values_equal` for the final
///   check — preserving epsilon semantics on both paths and requiring
///   `values` to be pre-sorted (`conversion.rs::convert_in` guarantees this).
fn in_list_matches(field_val: &Value, values: &[Value]) -> bool {
    if values.len() > IN_BINARY_SEARCH_THRESHOLD {
        debug_assert!(
            values
                .windows(2)
                .all(|w| json_value_cmp(&w[0], &w[1]) != std::cmp::Ordering::Greater),
            "Condition::In values must be pre-sorted by json_value_cmp"
        );
        // Lower-bound: first position where probe is NOT less than field_val.
        let pos = values
            .partition_point(|probe| json_value_cmp(probe, field_val) == std::cmp::Ordering::Less);
        // Check the candidate at pos and its predecessor for epsilon-equal floats.
        let check = |i: usize| values.get(i).is_some_and(|v| values_equal(v, field_val));
        check(pos) || pos > 0 && check(pos - 1)
    } else {
        values.iter().any(|val| values_equal(field_val, val))
    }
}

/// Evaluates array-level conditions (single, any, all).
fn match_array_condition(payload: &Value, field: &str, values: &[Value], mode: ArrayMode) -> bool {
    get_field(payload, field).is_some_and(|v| match v {
        Value::Array(arr) => match mode {
            ArrayMode::Single => values
                .first()
                .is_some_and(|val| arr.iter().any(|e| values_equal(e, val))),
            ArrayMode::Any => values
                .iter()
                .any(|val| arr.iter().any(|e| values_equal(e, val))),
            ArrayMode::All => values
                .iter()
                .all(|val| arr.iter().any(|e| values_equal(e, val))),
        },
        _ => false,
    })
}

/// Array condition matching mode.
#[derive(Clone, Copy)]
enum ArrayMode {
    Single,
    Any,
    All,
}

/// Evaluates geospatial conditions (distance, bounding box).
fn match_geo_distance(
    payload: &Value,
    field: &str,
    lat: f64,
    lng: f64,
    operator: crate::velesql::CompareOp,
    threshold: f64,
) -> bool {
    get_field(payload, field).is_some_and(|v| {
        extract_geo_point(v).is_some_and(|(plat, plng)| {
            let dist = haversine_distance_m(plat, plng, lat, lng);
            compare_geo_distance(dist, threshold, operator)
        })
    })
}

/// Evaluates a geospatial bounding box check.
fn match_geo_bbox(
    payload: &Value,
    field: &str,
    lat_min: f64,
    lng_min: f64,
    lat_max: f64,
    lng_max: f64,
) -> bool {
    get_field(payload, field).is_some_and(|v| {
        extract_geo_point(v).is_some_and(|(plat, plng)| {
            plat >= lat_min && plat <= lat_max && plng >= lng_min && plng <= lng_max
        })
    })
}

/// Extracts `(lat, lng)` from a JSON object with `"lat"` and `"lng"` keys.
fn extract_geo_point(v: &Value) -> Option<(f64, f64)> {
    let lat = v.get("lat").and_then(Value::as_f64)?;
    let lng = v.get("lng").and_then(Value::as_f64)?;
    Some((lat, lng))
}

impl Condition {
    /// Evaluates the condition against a payload.
    #[must_use]
    pub fn matches(&self, payload: &Value) -> bool {
        match self {
            Self::Eq { field, value } => {
                get_field(payload, field).is_some_and(|v| values_equal(v, value))
            }
            Self::Neq { field, value } => {
                get_field(payload, field).is_none_or(|v| !values_equal(v, value))
            }
            Self::Gt { field, value } => get_field(payload, field)
                .is_some_and(|v| compare_values(v, value).is_some_and(std::cmp::Ordering::is_gt)),
            Self::Gte { field, value } => get_field(payload, field)
                .is_some_and(|v| compare_values(v, value).is_some_and(std::cmp::Ordering::is_ge)),
            Self::Lt { field, value } => get_field(payload, field)
                .is_some_and(|v| compare_values(v, value).is_some_and(std::cmp::Ordering::is_lt)),
            Self::Lte { field, value } => get_field(payload, field)
                .is_some_and(|v| compare_values(v, value).is_some_and(std::cmp::Ordering::is_le)),
            Self::In { field, values } => {
                get_field(payload, field).is_some_and(|v| in_list_matches(v, values))
            }
            Self::Contains { field, value } => get_field(payload, field)
                .is_some_and(|v| v.as_str().is_some_and(|s| s.contains(value.as_str()))),
            Self::IsNull { field } => get_field(payload, field).is_none_or(Value::is_null),
            Self::IsNotNull { field } => get_field(payload, field).is_some_and(|v| !v.is_null()),
            Self::And { conditions } => conditions.iter().all(|c| c.matches(payload)),
            Self::Or { conditions } => conditions.iter().any(|c| c.matches(payload)),
            Self::Not { condition } => !condition.matches(payload),
            Self::Like { field, pattern } => get_field(payload, field)
                .is_some_and(|v| v.as_str().is_some_and(|s| like_match(s, pattern, false))),
            Self::ILike { field, pattern } => get_field(payload, field)
                .is_some_and(|v| v.as_str().is_some_and(|s| like_match(s, pattern, true))),
            Self::ArrayContains { field, value } => match_array_condition(
                payload,
                field,
                std::slice::from_ref(value),
                ArrayMode::Single,
            ),
            Self::ArrayContainsAny { field, values } => {
                match_array_condition(payload, field, values, ArrayMode::Any)
            }
            Self::ArrayContainsAll { field, values } => {
                match_array_condition(payload, field, values, ArrayMode::All)
            }
            Self::GeoDistance {
                field,
                lat,
                lng,
                operator,
                threshold,
            } => match_geo_distance(payload, field, *lat, *lng, *operator, *threshold),
            Self::GeoBbox {
                field,
                lat_min,
                lng_min,
                lat_max,
                lng_max,
            } => match_geo_bbox(payload, field, *lat_min, *lng_min, *lat_max, *lng_max),
        }
    }
}

/// Gets a field from a JSON payload, supporting dot notation for nested fields.
fn get_field<'a>(payload: &'a Value, field: &str) -> Option<&'a Value> {
    let mut current = payload;
    for part in field.split('.') {
        current = current.get(part)?;
    }
    Some(current)
}

/// Compares two JSON values for equality.
fn values_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Null, Value::Null) => true,
        (Value::Bool(a), Value::Bool(b)) => a == b,
        (Value::Number(a), Value::Number(b)) => {
            // Compare as f64 for numeric comparison
            a.as_f64()
                .zip(b.as_f64())
                .is_some_and(|(a, b)| (a - b).abs() < f64::EPSILON)
        }
        (Value::String(a), Value::String(b)) => a == b,
        (Value::Array(a), Value::Array(b)) => a == b,
        (Value::Object(a), Value::Object(b)) => a == b,
        _ => false,
    }
}

/// Compares two JSON values for ordering predicates (Gt, Gte, Lt, Lte).
///
/// Returns `None` for incompatible types (e.g. Number vs String, any vs Null)
/// and for NaN number values. Callers must treat `None` as false — matching
/// SQL three-valued logic where `NULL op X` yields UNKNOWN (not true).
fn compare_values(a: &Value, b: &Value) -> Option<std::cmp::Ordering> {
    match (a, b) {
        (Value::Number(a), Value::Number(b)) => a
            .as_f64()
            .zip(b.as_f64())
            .and_then(|(fa, fb)| fa.partial_cmp(&fb)),
        (Value::String(a), Value::String(b)) => Some(a.cmp(b)),
        _ => None,
    }
}

/// SQL LIKE pattern matching implementation.
///
/// Supports:
/// - `%` matches zero or more characters
/// - `_` matches exactly one character
/// - `\%` matches a literal `%`
/// - `\_` matches a literal `_`
///
/// # Arguments
///
/// * `text` - The string to match against
/// * `pattern` - The SQL LIKE pattern
/// * `case_insensitive` - If true, performs case-insensitive matching (ILIKE)
fn like_match(text: &str, pattern: &str, case_insensitive: bool) -> bool {
    if pattern.len() > LIKE_MAX_PATTERN_BYTES {
        global_guardrails_metrics().record_like_guardrail_rejected();
        return false;
    }

    if text.len().saturating_mul(pattern.len().max(1)) > LIKE_MAX_DYN_OPS {
        global_guardrails_metrics().record_like_guardrail_rejected();
        return false;
    }

    if case_insensitive {
        like_match_impl(
            text.to_lowercase().as_bytes(),
            pattern.to_lowercase().as_bytes(),
        )
    } else {
        like_match_impl(text.as_bytes(), pattern.as_bytes())
    }
}

#[derive(Clone, Copy)]
enum Token {
    AnySeq,
    AnyOne,
    Literal(u8),
}

fn tokenize_like_pattern(pattern: &[u8]) -> Vec<Token> {
    let mut out = Vec::with_capacity(pattern.len());
    let mut i = 0;
    while i < pattern.len() {
        match pattern[i] {
            b'\\' if i + 1 < pattern.len() => {
                out.push(Token::Literal(pattern[i + 1]));
                i += 2;
            }
            b'%' => {
                if !matches!(out.last(), Some(Token::AnySeq)) {
                    out.push(Token::AnySeq);
                }
                i += 1;
            }
            b'_' => {
                out.push(Token::AnyOne);
                i += 1;
            }
            c => {
                out.push(Token::Literal(c));
                i += 1;
            }
        }
    }
    out
}

/// LIKE matching using rolling DP (`O(text_len * token_len)` time, `O(token_len)` memory).
fn like_match_impl(text: &[u8], pattern: &[u8]) -> bool {
    let tokens = tokenize_like_pattern(pattern);
    let n = tokens.len();

    let mut prev = vec![false; n + 1];
    prev[0] = true;
    for (j, tok) in tokens.iter().enumerate() {
        if matches!(tok, Token::AnySeq) {
            prev[j + 1] = prev[j];
        } else {
            break;
        }
    }

    let mut curr = vec![false; n + 1];
    for &ch in text {
        curr.fill(false);
        for (j, tok) in tokens.iter().enumerate() {
            curr[j + 1] = match tok {
                Token::AnySeq => curr[j] || prev[j + 1],
                Token::AnyOne => prev[j],
                Token::Literal(c) => prev[j] && ch == *c,
            };
        }
        std::mem::swap(&mut prev, &mut curr);
    }

    prev[n]
}

/// Haversine great-circle distance. Returns distance in **meters** (WGS-84 mean radius).
///
/// Kept local so this module compiles without the `persistence` feature
/// (which gates `column_store::haversine`).
fn haversine_distance_m(lat1: f64, lng1: f64, lat2: f64, lng2: f64) -> f64 {
    const EARTH_RADIUS_M: f64 = 6_371_000.0;
    let (lat1, lng1) = (lat1.to_radians(), lng1.to_radians());
    let (lat2, lng2) = (lat2.to_radians(), lng2.to_radians());
    let dlat = lat2 - lat1;
    let dlng = lng2 - lng1;
    let a = (dlat / 2.0).sin().powi(2) + lat1.cos() * lat2.cos() * (dlng / 2.0).sin().powi(2);
    EARTH_RADIUS_M * 2.0 * a.sqrt().atan2((1.0 - a).sqrt())
}

/// Applies a comparison operator to a geo-distance value and threshold.
fn compare_geo_distance(dist: f64, threshold: f64, op: crate::velesql::CompareOp) -> bool {
    use crate::velesql::CompareOp;
    match op {
        CompareOp::Eq => (dist - threshold).abs() < f64::EPSILON,
        CompareOp::NotEq => (dist - threshold).abs() >= f64::EPSILON,
        CompareOp::Gt => dist > threshold,
        CompareOp::Gte => dist >= threshold,
        CompareOp::Lt => dist < threshold,
        CompareOp::Lte => dist <= threshold,
    }
}

#[cfg(test)]
mod tests {
    use crate::filter::Condition;
    use serde_json::json;

    // Null field must never match any ordering predicate — SQL three-valued logic.
    // Previously compare_values returned 0 for incompatible types, so `null >= N`
    // and `null <= N` spuriously returned true.
    #[test]
    fn null_field_never_matches_ordering_predicates() {
        let p = json!({"price": null});
        let n = json!(100);
        let field = "price".to_string();
        assert!(!Condition::Gt {
            field: field.clone(),
            value: n.clone()
        }
        .matches(&p));
        assert!(!Condition::Gte {
            field: field.clone(),
            value: n.clone()
        }
        .matches(&p));
        assert!(!Condition::Lt {
            field: field.clone(),
            value: n.clone()
        }
        .matches(&p));
        assert!(!Condition::Lte {
            field: field.clone(),
            value: n
        }
        .matches(&p));
    }

    // A boolean field must not match numeric ordering predicates.
    #[test]
    fn bool_field_never_matches_numeric_ordering() {
        let p = json!({"active": true});
        let n = json!(1);
        let field = "active".to_string();
        assert!(!Condition::Gt {
            field: field.clone(),
            value: n.clone()
        }
        .matches(&p));
        assert!(!Condition::Gte {
            field: field.clone(),
            value: n.clone()
        }
        .matches(&p));
        assert!(!Condition::Lt {
            field: field.clone(),
            value: n.clone()
        }
        .matches(&p));
        assert!(!Condition::Lte {
            field: field.clone(),
            value: n
        }
        .matches(&p));
    }

    // A string field must not match a numeric operand.
    #[test]
    fn string_field_never_matches_number_operand() {
        let p = json!({"name": "alice"});
        let n = json!(100);
        let field = "name".to_string();
        assert!(!Condition::Gt {
            field: field.clone(),
            value: n.clone()
        }
        .matches(&p));
        assert!(!Condition::Gte {
            field: field.clone(),
            value: n.clone()
        }
        .matches(&p));
        assert!(!Condition::Lt {
            field: field.clone(),
            value: n.clone()
        }
        .matches(&p));
        assert!(!Condition::Lte {
            field: field.clone(),
            value: n
        }
        .matches(&p));
    }

    // Numeric ordering works correctly for same-type comparisons.
    #[test]
    fn numeric_ordering_same_type() {
        let p = json!({"price": 50});
        let f = "price".to_string();
        assert!(Condition::Gt {
            field: f.clone(),
            value: json!(10)
        }
        .matches(&p));
        assert!(Condition::Gte {
            field: f.clone(),
            value: json!(50)
        }
        .matches(&p));
        assert!(!Condition::Gt {
            field: f.clone(),
            value: json!(50)
        }
        .matches(&p));
        assert!(Condition::Lt {
            field: f.clone(),
            value: json!(100)
        }
        .matches(&p));
        assert!(Condition::Lte {
            field: f.clone(),
            value: json!(50)
        }
        .matches(&p));
        assert!(!Condition::Lt {
            field: f.clone(),
            value: json!(50)
        }
        .matches(&p));
    }

    // String ordering works correctly for same-type comparisons.
    #[test]
    fn string_ordering_same_type() {
        let p = json!({"name": "bob"});
        let f = "name".to_string();
        assert!(Condition::Gt {
            field: f.clone(),
            value: json!("alice")
        }
        .matches(&p));
        assert!(Condition::Gte {
            field: f.clone(),
            value: json!("bob")
        }
        .matches(&p));
        assert!(Condition::Lt {
            field: f.clone(),
            value: json!("charlie")
        }
        .matches(&p));
        assert!(Condition::Lte {
            field: f.clone(),
            value: json!("bob")
        }
        .matches(&p));
    }
}
