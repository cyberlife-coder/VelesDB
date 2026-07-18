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

#[test]
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
fn parse_id_fields_rejects_a_non_numeric_id_string() {
    let mut value = json!({"fragment_id": "not-a-number"});

    let err = parse_id_fields(&mut value).unwrap_err();

    assert!(
        err.contains("not-a-number"),
        "error names the offending value: {err}"
    );
}

#[test]
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
fn parse_fragment_id_strings_is_a_no_op_without_a_fragments_array() {
    let mut request = json!({"query": "q"});

    parse_fragment_id_strings(&mut request).expect("no fragments key is fine");

    assert_eq!(request, json!({"query": "q"}));
}

#[test]
fn round_trip_stringify_then_parse_is_identity_for_id_keys() {
    let original = json!({"fragment_id": 42, "fragment_ids": [1, 2, 3]});
    let mut value = original.clone();

    stringify_id_fields(&mut value);
    parse_id_fields(&mut value).expect("round-trip ids are always valid decimals");

    assert_eq!(value, original);
}
