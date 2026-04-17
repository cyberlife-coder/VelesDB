//! Value and parameter resolution for the VelesQL executor (S4-13).
//!
//! Converts [`velesdb_core::velesql::Value`] to `serde_json::Value` while
//! resolving `$param` placeholders against the params map. Keeps the rules
//! simple and WASM-safe (no clock dependency for temporal, no subqueries).

use std::collections::HashMap;

use velesdb_core::velesql::{Value, VectorExpr};

/// Params map for VelesQL queries (`$name` → JSON value).
pub(crate) type Params = HashMap<String, serde_json::Value>;

/// Parses the raw JSON params blob into a [`Params`] map.
///
/// Accepts:
/// - `None` -> empty map
/// - `Some("{}")` -> empty map
/// - `Some("{\"k\": 10}")` -> `{"k" -> 10}`
///
/// Invalid JSON returns a descriptive error string for the FFI layer.
pub(crate) fn parse_params(raw: Option<&str>) -> Result<Params, String> {
    let Some(raw) = raw else {
        return Ok(Params::new());
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(Params::new());
    }
    serde_json::from_str(trimmed).map_err(|e| format!("Invalid params JSON: {e}"))
}

/// Resolves a VelesQL [`Value`] into a concrete `serde_json::Value`.
///
/// Parameters are looked up in `params`; missing parameters produce an error.
/// Subqueries and temporal expressions beyond `NOW()` are rejected here
/// because WASM has neither a query planner nor side-effect isolation to
/// evaluate them safely.
pub(crate) fn resolve_value(v: &Value, params: &Params) -> Result<serde_json::Value, String> {
    match v {
        Value::Integer(i) => Ok(serde_json::json!(i)),
        Value::UnsignedInteger(u) => Ok(serde_json::json!(u)),
        Value::Float(f) => Ok(serde_json::json!(f)),
        Value::String(s) => Ok(serde_json::Value::String(s.clone())),
        Value::Boolean(b) => Ok(serde_json::Value::Bool(*b)),
        Value::Null => Ok(serde_json::Value::Null),
        Value::Parameter(name) => params
            .get(name.as_str())
            .cloned()
            .ok_or_else(|| format!("Parameter ${name} is not bound")),
        Value::Temporal(expr) => Ok(serde_json::json!(expr.to_epoch_seconds())),
        Value::Subquery(_) => Err("Subqueries are not supported in WASM".to_string()),
        // Defensive: `Value` is `#[non_exhaustive]` for forward-compatibility.
        _ => Err(format!("Unsupported VelesQL value variant in WASM: {v:?}")),
    }
}

/// Resolves a vector expression to `Vec<f32>`.
///
/// Literal vectors are returned as-is. Parameter-bound vectors must be bound
/// to a JSON array of numbers in `params`.
pub(crate) fn resolve_vector(expr: &VectorExpr, params: &Params) -> Result<Vec<f32>, String> {
    match expr {
        VectorExpr::Literal(v) => Ok(v.clone()),
        VectorExpr::Parameter(name) => {
            let value = params
                .get(name.as_str())
                .ok_or_else(|| format!("Vector parameter ${name} is not bound"))?;
            json_to_f32_vec(value, name)
        }
        // Defensive: `VectorExpr` is `#[non_exhaustive]`.
        _ => Err(format!("Unsupported VectorExpr variant in WASM: {expr:?}")),
    }
}

/// Converts a JSON array of numbers into `Vec<f32>`.
fn json_to_f32_vec(value: &serde_json::Value, name: &str) -> Result<Vec<f32>, String> {
    let arr = value
        .as_array()
        .ok_or_else(|| format!("Parameter ${name} must be a JSON array of numbers"))?;
    let mut out = Vec::with_capacity(arr.len());
    for (idx, item) in arr.iter().enumerate() {
        let as_f64 = item
            .as_f64()
            .ok_or_else(|| format!("Parameter ${name}[{idx}] is not a number"))?;
        let narrowed = as_f64 as f32;
        if !narrowed.is_finite() && as_f64.is_finite() {
            return Err(format!("Parameter ${name}[{idx}] overflows f32"));
        }
        out.push(narrowed);
    }
    Ok(out)
}

/// Compares two JSON values as "equal" according to VelesQL semantics.
///
/// Numeric values compare by numeric equality across integer / float.
/// Strings, booleans, and null are compared by structural equality.
pub(crate) fn json_values_equal(left: &serde_json::Value, right: &serde_json::Value) -> bool {
    if let (Some(a), Some(b)) = (left.as_f64(), right.as_f64()) {
        return (a - b).abs() < f64::EPSILON;
    }
    left == right
}

/// Compares two JSON values numerically, when both can be interpreted as numbers.
///
/// Returns `None` when either value is not numeric (caller decides what "false"
/// means for non-numeric operands on a `>`/`<` comparison).
pub(crate) fn json_values_cmp(
    left: &serde_json::Value,
    right: &serde_json::Value,
) -> Option<std::cmp::Ordering> {
    match (left.as_f64(), right.as_f64()) {
        (Some(a), Some(b)) => a.partial_cmp(&b),
        _ => {
            // Fall back to lexicographic compare for strings.
            match (left.as_str(), right.as_str()) {
                (Some(a), Some(b)) => Some(a.cmp(b)),
                _ => None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params_from(json: &str) -> Params {
        parse_params(Some(json)).expect("test: parse params")
    }

    #[test]
    fn test_parse_params_none() {
        assert!(parse_params(None).expect("test: none").is_empty());
    }

    #[test]
    fn test_parse_params_empty_string() {
        assert!(parse_params(Some("")).expect("test: empty").is_empty());
    }

    #[test]
    fn test_parse_params_empty_object() {
        assert!(parse_params(Some("{}")).expect("test: {}").is_empty());
    }

    #[test]
    fn test_parse_params_valid_object() {
        let p = params_from(r#"{"k": 10, "s": "x"}"#);
        assert_eq!(p.get("k"), Some(&serde_json::json!(10)));
        assert_eq!(p.get("s"), Some(&serde_json::json!("x")));
    }

    #[test]
    fn test_parse_params_invalid_returns_error() {
        let err = parse_params(Some("not json"));
        assert!(err.is_err());
        assert!(
            err.expect_err("test: err").contains("Invalid params JSON"),
            "error should mention 'Invalid params JSON'"
        );
    }

    #[test]
    fn test_resolve_value_integer() {
        let v = resolve_value(&Value::Integer(42), &Params::new()).expect("test: int");
        assert_eq!(v, serde_json::json!(42));
    }

    #[test]
    fn test_resolve_value_string() {
        let v = resolve_value(&Value::String("x".to_string()), &Params::new())
            .expect("test: string");
        assert_eq!(v, serde_json::json!("x"));
    }

    #[test]
    fn test_resolve_value_null() {
        let v = resolve_value(&Value::Null, &Params::new()).expect("test: null");
        assert_eq!(v, serde_json::Value::Null);
    }

    #[test]
    fn test_resolve_value_parameter_bound() {
        let p = params_from(r#"{"x": 42}"#);
        let v = resolve_value(&Value::Parameter("x".to_string()), &p).expect("test: bound");
        assert_eq!(v, serde_json::json!(42));
    }

    #[test]
    fn test_resolve_value_parameter_unbound_errors() {
        let err = resolve_value(&Value::Parameter("missing".to_string()), &Params::new());
        assert!(err.is_err());
        let msg = err.expect_err("test: err");
        assert!(msg.contains("$missing"), "error should mention $missing");
    }

    #[test]
    fn test_resolve_vector_literal() {
        let v = resolve_vector(&VectorExpr::Literal(vec![1.0, 2.0]), &Params::new())
            .expect("test: literal");
        assert_eq!(v, vec![1.0, 2.0]);
    }

    #[test]
    fn test_resolve_vector_param_bound() {
        let p = params_from(r#"{"q": [0.5, 0.25]}"#);
        let v = resolve_vector(&VectorExpr::Parameter("q".to_string()), &p)
            .expect("test: bound");
        assert_eq!(v.len(), 2);
        assert!((v[0] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn test_resolve_vector_param_unbound_errors() {
        let err = resolve_vector(&VectorExpr::Parameter("q".to_string()), &Params::new());
        assert!(err.is_err());
    }

    #[test]
    fn test_resolve_vector_param_not_array_errors() {
        let p = params_from(r#"{"q": "not an array"}"#);
        let err = resolve_vector(&VectorExpr::Parameter("q".to_string()), &p);
        assert!(err.is_err());
        assert!(err.expect_err("test: err").contains("must be a JSON array"));
    }

    #[test]
    fn test_resolve_vector_param_non_number_element_errors() {
        let p = params_from(r#"{"q": [1.0, "nope", 3.0]}"#);
        let err = resolve_vector(&VectorExpr::Parameter("q".to_string()), &p);
        assert!(err.is_err());
    }

    #[test]
    fn test_json_values_equal_mixed_numeric() {
        assert!(json_values_equal(
            &serde_json::json!(42),
            &serde_json::json!(42.0)
        ));
    }

    #[test]
    fn test_json_values_equal_strings() {
        assert!(json_values_equal(
            &serde_json::json!("a"),
            &serde_json::json!("a")
        ));
        assert!(!json_values_equal(
            &serde_json::json!("a"),
            &serde_json::json!("b")
        ));
    }

    #[test]
    fn test_json_values_cmp_numeric() {
        let a = serde_json::json!(1);
        let b = serde_json::json!(2.5);
        assert_eq!(json_values_cmp(&a, &b), Some(std::cmp::Ordering::Less));
    }

    #[test]
    fn test_json_values_cmp_strings() {
        let a = serde_json::json!("apple");
        let b = serde_json::json!("banana");
        assert_eq!(json_values_cmp(&a, &b), Some(std::cmp::Ordering::Less));
    }

    #[test]
    fn test_json_values_cmp_incompatible_returns_none() {
        let a = serde_json::json!(true);
        let b = serde_json::json!(42);
        assert_eq!(json_values_cmp(&a, &b), None);
    }
}
