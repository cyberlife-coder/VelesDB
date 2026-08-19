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
    let v = resolve_value(&Value::String("x".to_string()), &Params::new()).expect("test: string");
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
    let v = resolve_vector(&VectorExpr::Parameter("q".to_string()), &p).expect("test: bound");
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
