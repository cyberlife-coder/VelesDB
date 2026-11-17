//! The crate denies `unwrap`/`expect` in tests too: results are read
//! through `ok()`.
use serde_json::json;

use super::{integral_numbers, json_number, resolve_transcript_input};

/// What napi hands over for a JS whole number past `u32::MAX`: a float.
fn napi_float(n: f64) -> serde_json::Value {
    serde_json::Number::from_f64(n).map_or(serde_json::Value::Null, serde_json::Value::Number)
}

#[test]
fn a_whole_number_past_u32_is_an_integer_again() {
    let input = json!({ "ts": napi_float(5_000_000_000.0), "neg": napi_float(-5_000_000_000.0) });
    assert_eq!(
        integral_numbers(input),
        json!({ "ts": 5_000_000_000_i64, "neg": -5_000_000_000_i64 })
    );
}

#[test]
fn nested_arrays_and_objects_are_restored_too() {
    let input = json!({ "ids": [napi_float(9_007_199_254_740_991.0)], "o": { "n": napi_float(4_294_967_296.0) } });
    assert_eq!(
        integral_numbers(input),
        json!({ "ids": [9_007_199_254_740_991_i64], "o": { "n": 4_294_967_296_i64 } })
    );
}

#[test]
fn fractions_and_numbers_past_2_pow_53_stay_floats() {
    let fraction = napi_float(1.5);
    let unsafe_whole = napi_float(9_007_199_254_740_992.0);
    assert_eq!(integral_numbers(fraction.clone()), fraction);
    assert_eq!(integral_numbers(unsafe_whole.clone()), unsafe_whole);
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
