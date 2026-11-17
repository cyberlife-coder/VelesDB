use super::*;
use serde_json::json;

#[test]
fn stringify_id_fields_rewrites_known_id_keys_only() {
    let mut value = json!({
        "fragment_id": 42,
        "content_hash": 18_446_744_073_709_551_615u64,
        "fragment_ids": [1, 2, 3],
        "risk": "low",
        "nested": {"memory_id": 7},
    });

    stringify_id_fields(&mut value);

    assert_eq!(value["fragment_id"], json!("42"));
    assert_eq!(value["content_hash"], json!("18446744073709551615"));
    assert_eq!(value["fragment_ids"], json!(["1", "2", "3"]));
    assert_eq!(
        value["risk"],
        json!("low"),
        "non-id fields pass through untouched"
    );
    assert_eq!(value["nested"]["memory_id"], json!("7"));
}

/// What lets the JS bindings hand their decimal-string input straight to
/// serde: every id a binding READS (`WorkingContext`'s evidence and
/// decisions, `CompileRequest`'s `fragments[].id`) deserializes from the
/// exact form [`stringify_id_fields`] writes. A string-to-number pre-pass
/// over the JSON tree would only duplicate the domain's own id rule.
#[test]
fn a_working_context_in_its_js_wire_form_deserializes_unchanged() {
    let numeric = json!({
        "goal": "resume",
        "verified_facts": [{
            "text": "fact",
            "source": {"fragment_id": 18_446_744_073_709_551_615u64, "handle": "ctx://source/1", "memory_id": 9_007_199_254_740_993u64},
        }],
        "decisions": [{"fragment_id": 9_007_199_254_740_993u64, "rule_id": "media.atomic"}],
        "exact_evidence": [{"fragment_id": 42, "handle": "ctx://source/42", "memory_id": 7}],
    });
    let original: crate::context::WorkingContext =
        serde_json::from_value(numeric).expect("numeric wire form");
    let mut stringified = serde_json::to_value(&original).expect("serializes");
    stringify_id_fields(&mut stringified);

    let parsed: crate::context::WorkingContext =
        serde_json::from_value(stringified).expect("the JS wire form is accepted as is");

    assert_eq!(
        serde_json::to_value(&parsed).expect("serializes"),
        serde_json::to_value(&original).expect("serializes"),
    );
}

#[test]
fn a_compile_request_takes_its_fragment_ids_as_decimal_strings() {
    let request: crate::context::CompileRequest = serde_json::from_value(json!({
        "query": "q",
        "token_budget": 100,
        "fragments": [
            {"id": "18446744073709551615", "content": "a"},
            {"content": "b"},
        ],
    }))
    .expect("decimal-string fragment ids are the JS wire form");

    assert_eq!(request.fragments[0].id, Some(u64::MAX));
    assert_eq!(request.fragments[1].id, None);
}

/// One id rule on every surface: the MCP tools, Node and WASM all read a
/// context id through the domain deserializer, so they agree on the edge
/// cases too — surrounding whitespace is trimmed, a non-numeric id names
/// itself in the error.
#[test]
fn a_context_id_string_follows_the_crate_wide_id_rule() {
    let source: crate::context::SourceReference =
        serde_json::from_value(json!({"fragment_id": " 42 ", "handle": "h", "memory_id": " 7 "}))
            .expect("the crate-wide id rule trims, optional ids included");
    assert_eq!((source.fragment_id, source.memory_id), (42, Some(7)));
    let fragment: ContextFragment = serde_json::from_value(json!({"id": " 5 ", "content": "a"}))
        .expect("a fragment id takes the same rule");
    assert_eq!(fragment.id, Some(5));

    let err = serde_json::from_value::<crate::context::WorkingContext>(json!({
        "exact_evidence": [{"fragment_id": "1", "handle": "h", "memory_id": "not-a-number"}],
    }))
    .expect_err("a non-numeric id is refused");
    assert!(err.to_string().contains("not-a-number"), "{err}");
}

/// The claim the three JS bindings rely on when they call
/// [`stringify_id_fields`] at the ROOT of the `load_working_context`
/// envelope instead of on `working` alone: the walk descends by KEY NAME, so
/// wrapping the working context one level deeper cannot hide its ids.
///
/// Checked rather than assumed — the previous code stringified `working`
/// directly, and "it still works one level down" is exactly the kind of
/// property a refactor breaks without any compiler complaint.
#[test]
fn stringify_id_fields_reaches_ids_nested_under_the_load_envelope() {
    let mut envelope = json!({
        "found": true,
        "other_sessions": ["task-1234"],
        "working": {
            "goal": "ship the envelope",
            "decisions": [
                {"fragment_id": 18_446_744_073_709_551_615u64, "rule_id": "media.atomic"},
            ],
            "exact_evidence": [
                {"fragment_id": 42, "memory_id": 7, "handle": "ctx://source/42"},
            ],
        },
    });

    stringify_id_fields(&mut envelope);

    assert_eq!(
        envelope["working"]["decisions"][0]["fragment_id"],
        json!("18446744073709551615"),
        "a decision id two levels under the envelope root must still be stringified"
    );
    assert_eq!(
        envelope["working"]["exact_evidence"][0]["fragment_id"],
        json!("42")
    );
    assert_eq!(
        envelope["working"]["exact_evidence"][0]["memory_id"],
        json!("7")
    );
    assert_eq!(
        envelope["found"],
        json!(true),
        "the envelope's own scalars pass through untouched"
    );
    assert_eq!(envelope["other_sessions"], json!(["task-1234"]));
}

// --- the deprecated string-to-number walks keep their behaviour -----------

#[test]
#[expect(
    deprecated,
    reason = "the deprecated helpers keep working until removed"
)]
fn parse_id_fields_is_the_inverse_of_stringify() {
    let mut value = json!({
        "fragment_id": "42",
        "fragment_ids": ["1", "2", "3"],
        "nested": {"memory_id": "7"},
    });

    parse_id_fields(&mut value).expect("valid decimal ids");

    assert_eq!(value["fragment_id"], json!(42));
    assert_eq!(value["fragment_ids"], json!([1, 2, 3]));
    assert_eq!(value["nested"]["memory_id"], json!(7));
}

#[test]
#[expect(
    deprecated,
    reason = "the deprecated helpers keep working until removed"
)]
fn parse_id_fields_rejects_a_non_numeric_id_string() {
    let mut value = json!({"fragment_id": "not-a-number"});

    let err = parse_id_fields(&mut value).unwrap_err();

    assert!(
        err.to_string().contains("not-a-number"),
        "error names the offending value: {err}"
    );
    assert_eq!(err.text, "not-a-number");
}

#[test]
#[expect(
    deprecated,
    reason = "the deprecated helpers keep working until removed"
)]
fn parse_fragment_id_strings_rewrites_fragment_ids_only() {
    let mut request = json!({
        "fragments": [
            {"id": "18446744073709551615", "content": "a"},
            {"content": "b"},
        ],
    });

    parse_fragment_id_strings(&mut request).expect("valid ids");

    assert_eq!(
        request["fragments"][0]["id"],
        json!(18_446_744_073_709_551_615u64)
    );
    assert!(request["fragments"][1].get("id").is_none());
}

#[test]
#[expect(
    deprecated,
    reason = "the deprecated helpers keep working until removed"
)]
fn parse_fragment_id_strings_is_a_no_op_without_a_fragments_array() {
    let mut request = json!({"query": "q"});

    parse_fragment_id_strings(&mut request).expect("no fragments key is fine");

    assert_eq!(request, json!({"query": "q"}));
}

#[test]
#[expect(
    deprecated,
    reason = "the deprecated helpers keep working until removed"
)]
fn round_trip_stringify_then_parse_is_identity_for_id_keys() {
    let original = json!({"fragment_id": 42, "fragment_ids": [1, 2, 3]});
    let mut value = original.clone();

    stringify_id_fields(&mut value);
    parse_id_fields(&mut value).expect("round-trip ids are always valid decimals");

    assert_eq!(value, original);
}

// --- deserialize_optional_id (`ContextFragment.id` on the typed wire) -------

use crate::context::ContextFragment;

#[test]
fn deserialize_optional_id_accepts_a_json_number() {
    // The 0.8.0 wire form: a plain JSON number, including full-u64 range.
    let fragment: ContextFragment = serde_json::from_value(json!({
        "id": 18_446_744_073_709_551_615u64,
        "content": "a",
    }))
    .expect("numeric ids are the historical wire form and must keep working");

    assert_eq!(fragment.id, Some(u64::MAX));
}

#[test]
fn deserialize_optional_id_accepts_a_decimal_string() {
    let fragment: ContextFragment = serde_json::from_value(json!({
        "id": "9007199254740993",
        "content": "a",
    }))
    .expect("decimal-string ids are the JS-safe wire form");

    assert_eq!(fragment.id, Some(9_007_199_254_740_993));
}

#[test]
fn deserialize_optional_id_rejects_a_non_numeric_string_with_a_clear_message() {
    let err = serde_json::from_value::<ContextFragment>(json!({
        "id": "abc",
        "content": "a",
    }))
    .expect_err("a non-numeric id string cannot silently pass");

    let message = err.to_string();
    assert!(
        message.contains("abc") && message.contains("u64"),
        "the error names the offending value and the expected forms, \
         not an opaque untagged-enum mismatch: {message}"
    );
}

#[test]
fn deserialize_optional_id_rejects_a_non_integer_number_with_a_clear_message() {
    let err = serde_json::from_value::<ContextFragment>(json!({
        "id": 1.5,
        "content": "a",
    }))
    .expect_err("a fractional id cannot silently pass");

    let message = err.to_string();
    assert!(
        message.contains("1.5") && message.contains("u64"),
        "the error names the offending value and the expected forms, \
         not an opaque untagged-enum mismatch: {message}"
    );
}
