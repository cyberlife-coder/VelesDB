//! The one serializer for every Rust value that crosses into JavaScript as
//! data (#2332).
//!
//! `serde_wasm_bindgen`'s default serializer turns every map — a
//! `serde_json::Value::Object` payload, a `HashMap` of graph properties —
//! into an ES2015 `Map`, on which `record.id` is `undefined` and
//! `JSON.stringify` yields `{}`. Search records reached the TypeScript SDK
//! that way and it threw on every query that matched a row.
//! [`Serializer::json_compatible`] emits what `JSON.parse` would have
//! produced instead:
//!
//! - maps arrive as **plain objects**;
//! - an absent value (`None`, `serde_json::Value::Null`, `()`) arrives as
//!   **`null`**, which `JSON.stringify` keeps, not `undefined`, which it drops;
//! - the binding's id contract is unchanged: `u64` still arrives as a JS
//!   `number` and one above `Number.MAX_SAFE_INTEGER` still fails
//!   serialization, because `json_compatible` leaves
//!   `serialize_large_number_types_as_bigints` at `false` like the default.
//!
//! Every outbound `serde_wasm_bindgen` call in this crate goes through
//! [`to_js_value`]; `js_value_tests` fails on one that does not.

use serde::Serialize;
use serde_wasm_bindgen::Serializer;
use wasm_bindgen::JsValue;

/// Serializes `value` for JavaScript as plain objects, arrays and `null`.
pub(crate) fn to_js_value<T: Serialize + ?Sized>(
    value: &T,
) -> Result<JsValue, serde_wasm_bindgen::Error> {
    value.serialize(&Serializer::json_compatible())
}

// Reads the source tree with `std::fs`, which a wasm32 test run cannot.
#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "js_value_tests.rs"]
mod js_value_tests;
