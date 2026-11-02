use super::*;
use serde::Deserialize;
use serde_json::json;
use std::path::PathBuf;

/// `conformance/filter_wire_cases.json`, the fixture velesdb-core's
/// `tests/filter_wire_conformance.rs` runs through `Filter::matches`: every
/// core condition kind, with the payloads it must and must not select.
#[derive(Deserialize)]
struct Fixture {
    valid: Vec<ValidCase>,
    invalid: Vec<InvalidCase>,
}

#[derive(Deserialize)]
struct ValidCase {
    id: String,
    filter: Value,
    matches: Vec<Value>,
    rejects: Vec<Value>,
}

#[derive(Deserialize)]
struct InvalidCase {
    id: String,
    filter: Value,
}

fn fixture() -> Fixture {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../conformance/filter_wire_cases.json");
    let content = std::fs::read_to_string(path).expect("read filter wire fixture");
    serde_json::from_str(&content).expect("parse filter wire fixture")
}

#[test]
fn every_core_condition_kind_selects_what_core_selects() {
    let fixture = fixture();
    // The fixture names one case per kind; prove each is here.
    for kind in velesdb_core::CONDITION_TYPE_NAMES {
        assert!(
            fixture.valid.iter().any(|c| c
                .filter
                .to_string()
                .contains(&format!("\"type\":\"{kind}\""))),
            "fixture has no `{kind}` case"
        );
    }
    for case in fixture.valid {
        let filter = parse_filter(case.filter)
            .unwrap_or_else(|e| panic!("case {} must parse: {e}", case.id));
        for payload in &case.matches {
            assert!(
                filter.matches_point(0, Some(payload)),
                "case {} must match {payload}",
                case.id
            );
        }
        for payload in &case.rejects {
            assert!(
                !filter.matches_point(0, Some(payload)),
                "case {} must not match {payload}",
                case.id
            );
        }
    }
}

#[test]
fn a_filter_core_refuses_is_refused_with_cores_message() {
    for case in fixture().invalid {
        let err = parse_filter(case.filter.clone()).expect_err(&case.id);
        let core = velesdb_core::Filter::from_json_value(case.filter).expect_err(&case.id);
        assert_eq!(err, core, "case {} must carry core's message", case.id);
    }
    assert!(parse_filter(json!({}))
        .unwrap_err()
        .contains("missing field `condition`"));
    assert!(
        parse_filter(Value::Null).is_err(),
        "a null filter is refused"
    );
}

#[test]
fn test_nested_field() {
    let payload = json!({"user": {"name": "John"}});
    assert_eq!(
        get_nested_field(&payload, "user.name"),
        Some(&json!("John"))
    );
    assert_eq!(get_nested_field(&payload, "user.age"), None);
}
