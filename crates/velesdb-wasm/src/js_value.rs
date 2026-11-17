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
//! - a bare `u64` still arrives as a JS `number` and one above
//!   `Number.MAX_SAFE_INTEGER` fails serialization, because
//!   `json_compatible` leaves `serialize_large_number_types_as_bigints` at
//!   `false` like the default. That is right for a count, a depth or a
//!   payload integer, and wrong for an id: a point id travels as a
//!   [`PointId`], an edge id through `serde_id` directly, and both arrive as
//!   their exact decimal string.
//!
//! Every outbound `serde_wasm_bindgen` call in this crate goes through
//! [`to_js_value`]; `js_value_tests` fails on one that does not.

use serde::{Deserialize, Deserializer, Serialize};
use serde_wasm_bindgen::Serializer;
use velesdb_core::api_types::serde_id;
use wasm_bindgen::JsValue;

/// Serializes `value` for JavaScript as plain objects, arrays and `null`.
pub(crate) fn to_js_value<T: Serialize + ?Sized>(
    value: &T,
) -> Result<JsValue, serde_wasm_bindgen::Error> {
    value.serialize(&Serializer::json_compatible())
}

/// A point id as it crosses into JavaScript: its exact decimal string, as
/// REST sends `SearchResultResponse.id` and `/match` `bindings`
/// (`serde_id`). As a bare `u64` it would make [`to_js_value`] throw past
/// `Number.MAX_SAFE_INTEGER`, and a JSON number past it is rounded by
/// `JSON.parse`.
///
/// It reads back from a decimal string, a safe-integer number or a `BigInt`
/// (`serde_id::deserialize_id_from_string_or_number`), so a result handed
/// back to the binding (`hybrid_search_fuse`) keeps its exact id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PointId(pub(crate) u64);

impl Serialize for PointId {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serde_id::serialize_id_as_string(&self.0, serializer)
    }
}

impl<'de> Deserialize<'de> for PointId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        serde_id::deserialize_id_from_string_or_number(deserializer).map(Self)
    }
}

// Reads the source tree with `std::fs`, which a wasm32 test run cannot.
#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "js_value_tests.rs"]
mod js_value_tests;
