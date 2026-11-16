//! The crate denies `unwrap`/`expect` in tests too: results are read
//! through `ok()`.
use serde_json::json;

use super::{integral_numbers, to_metadata};

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
fn metadata_and_filters_carry_the_integer() {
    let metadata = to_metadata(Some(json!({ "ts": napi_float(5_000_000_000.0) })))
        .ok()
        .flatten();
    assert_eq!(
        metadata.and_then(|m| m.get("ts").cloned()),
        Some(json!(5_000_000_000_i64))
    );
}
