//! Metadata-filter wire contract shared with the TypeScript SDK.
//!
//! `conformance/filter_wire_cases.json` holds the exact filter JSON the SDK
//! sends (its agent-memory calls among them); the SDK test
//! `sdks/typescript/tests/filter-wire-contract.test.ts` asserts it emits those
//! shapes byte for byte. This test is the other half: every `valid` case must
//! deserialize as a core [`Filter`] and select exactly its `matches` payloads,
//! every `invalid` case must be refused — the same refusal `velesdb-server`
//! turns into a `400` on `/search` and `/points/scroll`.

use serde::Deserialize;
use serde_json::Value;
use std::path::PathBuf;
use velesdb_core::Filter;

#[derive(Debug, Deserialize)]
struct Fixture {
    valid: Vec<ValidCase>,
    invalid: Vec<InvalidCase>,
}

#[derive(Debug, Deserialize)]
struct ValidCase {
    id: String,
    filter: Value,
    matches: Vec<Value>,
    rejects: Vec<Value>,
}

#[derive(Debug, Deserialize)]
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
fn valid_filter_wire_cases_parse_and_select_exactly_their_payloads() {
    let fixture = fixture();
    assert!(!fixture.valid.is_empty(), "fixture has no valid cases");
    for case in fixture.valid {
        let filter = Filter::from_json_value(case.filter)
            .unwrap_or_else(|e| panic!("case {} must parse as a core Filter: {e}", case.id));
        for payload in &case.matches {
            assert!(
                filter.matches(payload),
                "case {} must match {payload}",
                case.id
            );
        }
        for payload in &case.rejects {
            assert!(
                !filter.matches(payload),
                "case {} must not match {payload}",
                case.id
            );
        }
    }
}

#[test]
fn invalid_filter_wire_cases_are_refused() {
    let fixture = fixture();
    assert!(!fixture.invalid.is_empty(), "fixture has no invalid cases");
    for case in fixture.invalid {
        assert!(
            Filter::from_json_value(case.filter).is_err(),
            "case {} must be refused by core",
            case.id
        );
    }
}

/// A VelesQL `id` column becomes an internal point-id field. A JSON filter
/// cannot name it — its `"id"` is the payload field — whether it is parsed
/// with `Filter::from_json_value` or deserialized directly, as
/// `velesdb-server` does.
#[test]
fn the_internal_point_id_field_is_refused_in_a_json_filter() {
    let leaf = serde_json::json!({"type": "eq", "field": "@velesql.point_id", "value": 1});
    for condition in [
        leaf.clone(),
        serde_json::json!({"type": "and", "conditions": [leaf.clone()]}),
        serde_json::json!({"type": "not", "condition": leaf}),
    ] {
        let filter = serde_json::json!({ "condition": condition });
        assert!(Filter::from_json_value(filter.clone()).is_err(), "{filter}");
        assert!(
            serde_json::from_value::<Filter>(filter.clone()).is_err(),
            "{filter}"
        );
    }
    let payload_id = Filter::from_json_value(serde_json::json!(
        {"condition": {"type": "eq", "field": "id", "value": 1}}
    ))
    .expect("test: `id` is a payload field in a JSON filter");
    assert!(payload_id.matches(&serde_json::json!({"id": 1})));
}
