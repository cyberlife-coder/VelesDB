//! Unit tests for the lenient MCP parameter deserializer.

use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Probe {
    #[serde(default, deserialize_with = "super::lenient")]
    limit: Option<usize>,
    #[serde(default, deserialize_with = "super::lenient")]
    flag: Option<bool>,
    #[serde(default, deserialize_with = "super::lenient")]
    map: Option<serde_json::Map<String, serde_json::Value>>,
}

#[test]
fn test_lenient_passes_proper_types_through_unchanged() {
    let probe: Probe = serde_json::from_value(serde_json::json!({
        "limit": 6, "flag": true, "map": {"k": "v"}
    }))
    .expect("proper types must keep deserializing");
    assert_eq!(probe.limit, Some(6));
    assert_eq!(probe.flag, Some(true));
    assert_eq!(
        probe.map.unwrap().get("k").and_then(|v| v.as_str()),
        Some("v")
    );
}

#[test]
fn test_lenient_parses_stringified_arguments() {
    let probe: Probe = serde_json::from_value(serde_json::json!({
        "limit": "6", "flag": "true", "map": "{\"k\": \"v\"}"
    }))
    .expect("stringified arguments must parse as JSON");
    assert_eq!(probe.limit, Some(6));
    assert_eq!(probe.flag, Some(true));
    assert_eq!(
        probe.map.unwrap().get("k").and_then(|v| v.as_str()),
        Some("v")
    );
}

#[test]
fn test_lenient_missing_fields_stay_none() {
    let probe: Probe =
        serde_json::from_value(serde_json::json!({})).expect("missing fields default");
    assert_eq!(probe.limit, None);
    assert_eq!(probe.flag, None);
    assert!(probe.map.is_none());
}

#[test]
fn test_lenient_rejects_garbage_strings_with_a_precise_error() {
    let err = serde_json::from_value::<Probe>(serde_json::json!({ "limit": "six" }))
        .expect_err("a non-JSON string must still fail");
    assert!(
        err.to_string().contains("JSON-encoded string"),
        "error must explain the string fallback: {err}"
    );
}
