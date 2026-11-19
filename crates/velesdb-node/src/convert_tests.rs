//! The crate denies `unwrap`/`expect` in tests too: results are read
//! through `ok()`.
use serde_json::json;

use super::{json_number, resolve_transcript_input, to_filters};

/// The `[INVALID_INPUT]` message `to_filters` refuses `filters` with.
fn refusal(filters: serde_json::Value) -> String {
    to_filters(filters)
        .err()
        .map(|e| e.reason)
        .unwrap_or_default()
}

#[test]
fn a_well_formed_filter_list_becomes_engine_filters() {
    let filters = to_filters(json!([{ "field": "ts", "op": "ge", "value": 5_000_000_000_i64 }]))
        .ok()
        .unwrap_or_default();
    let fields: Vec<_> = filters
        .iter()
        .map(|f| (f.field.as_str(), f.value.clone()))
        .collect();
    assert_eq!(fields, [("ts", json!(5_000_000_000_i64))]);
}

#[test]
fn a_malformed_filter_is_refused_naming_where_it_sits() {
    let cases = [
        (json!("eq"), "filters must be an array"),
        (json!([1]), "filters[0] must be an object"),
        (
            json!([{ "op": "eq", "value": 1 }]),
            "filters[0].field must be a string",
        ),
        (
            json!([{ "field": 1, "op": "eq", "value": 1 }]),
            "filters[0].field must be a string",
        ),
        (
            json!([{ "field": "x", "value": 1 }]),
            "filters[0].op must be a string",
        ),
        (
            json!([{ "field": "x", "op": "eq" }]),
            "filters[0].value is missing",
        ),
        (
            json!([{ "field": "x", "op": "near", "value": 1 }]),
            "invalid op 'near'",
        ),
    ];
    for (filters, expected) in cases {
        let message = refusal(filters.clone());
        assert!(
            message.starts_with("[INVALID_INPUT] "),
            "{filters} → {message}"
        );
        assert!(message.contains(expected), "{filters} → {message}");
    }
}

#[test]
fn a_null_transcript_or_path_is_absent_as_on_the_mcp_tool() {
    // `{ transcript: "...", path: null }` was refused as "both", and
    // `{ transcript: null, path: ... }` reached the path branch never.
    let resolved =
        resolve_transcript_input(json!({ "transcript": "User: hi", "path": null }), None)
            .ok()
            .and_then(|request| request.get("transcript").cloned());
    assert_eq!(resolved, Some(json!("User: hi")));

    // A `null` transcript with a path reads the path: with ingestion
    // disabled that is the ingest refusal, not "pass either, not both".
    let reason = resolve_transcript_input(json!({ "transcript": null, "path": "/t.md" }), None)
        .err()
        .map(|e| e.reason);
    assert!(
        reason
            .as_deref()
            .is_some_and(|r| !r.contains("not both") && !r.contains("exactly one")),
        "{reason:?}"
    );
}

#[test]
fn neither_both_nor_an_empty_transcript_is_accepted() {
    for request in [
        json!({ "query": "q" }),
        json!({ "transcript": "x", "path": "/t.md" }),
        json!({ "transcript": "" }),
        json!({ "transcript": 3 }),
    ] {
        let reason = resolve_transcript_input(request.clone(), None)
            .err()
            .map(|e| e.reason);
        assert!(
            reason
                .as_deref()
                .is_some_and(|r| r.contains("INVALID_INPUT")),
            "{request}: {reason:?}"
        );
    }
}

#[test]
fn json_number_keeps_whole_numbers_exact_and_refuses_non_finite() {
    assert_eq!(json_number(5_000_000_000.0), Some(5_000_000_000_i64.into()));
    assert_eq!(json_number(-3.0), Some((-3_i64).into()));
    assert_eq!(json_number(2.5).and_then(|n| n.as_f64()), Some(2.5));
    assert!(
        json_number(2.0_f64.powi(53)).is_some_and(|n| n.is_f64()),
        "past 2^53 - 1 stays a float"
    );
    assert_eq!(json_number(f64::NAN), None);
    assert_eq!(json_number(f64::INFINITY), None);
}
