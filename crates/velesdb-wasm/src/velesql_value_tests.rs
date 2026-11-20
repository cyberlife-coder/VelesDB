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
        err.expect_err("test: err")
            .to_string()
            .contains("Invalid params JSON"),
        "error should mention 'Invalid params JSON'"
    );
}
