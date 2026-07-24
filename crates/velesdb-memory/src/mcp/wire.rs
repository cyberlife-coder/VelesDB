//! Lenient wire-side deserialization for MCP tool parameters.
//!
//! Real MCP client harnesses (observed 2026-07-24 with Claude Code) can
//! serialize a non-string tool argument as a JSON-encoded STRING when their
//! own view of the advertised schema has degraded to "untyped" — `limit: 6`
//! arrives as `"6"`, `filter: {"project": "x"}` as `"{\"project\": \"x\"}"`,
//! and `save_working_context`'s whole `working` object as one escaped JSON
//! string. Rejecting those loses the call (and, for `save_working_context`,
//! silently loses a session handoff).
//!
//! [`lenient`] is the server-side half of the harness-proof wire contract
//! (the schema half is `crate::schema::inline_ref_only_properties`): accept
//! the properly-typed JSON value first, and fall back to parsing a string
//! argument AS JSON into the target type. Same defensive-interop class as
//! the #1468 string-or-number id contract in `crate::context::wire`.
//!
//! Never applied to genuinely string-typed parameters — a real string must
//! not be re-interpreted as JSON.

use serde::de::{DeserializeOwned, Error as DeError};
use serde::{Deserialize, Deserializer};

/// Deserializes `T` from either its proper JSON representation or from a
/// JSON-encoded string containing it. The non-string path is byte-for-byte
/// the plain serde behaviour; the error of the direct interpretation is the
/// one reported when the string fallback fails too, so a genuinely malformed
/// argument keeps a precise message.
pub(super) fn lenient<'de, T, D>(deserializer: D) -> Result<T, D::Error>
where
    T: DeserializeOwned,
    D: Deserializer<'de>,
{
    let raw = serde_json::Value::deserialize(deserializer)?;
    match raw {
        serde_json::Value::String(text) => serde_json::from_str(&text).map_err(|err| {
            DeError::custom(format!(
                "argument arrived as a JSON-encoded string and could not be \
                 parsed as the expected type: {err}"
            ))
        }),
        value => serde_json::from_value(value).map_err(DeError::custom),
    }
}

#[cfg(test)]
#[path = "wire_tests.rs"]
mod tests;
