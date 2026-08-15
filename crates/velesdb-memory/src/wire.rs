//! Lenient deserialization shared by canonical domain and MCP wire types.
//!
//! Real MCP client harnesses can serialize a non-string tool argument as a
//! JSON-encoded string when their view of the advertised schema has degraded
//! to "untyped". [`lenient`] accepts the properly typed JSON value first and
//! falls back to parsing a string argument as JSON into the target type.
//!
//! Never apply it to genuinely string-typed parameters: a real string must
//! not be reinterpreted as JSON.

use serde::de::{DeserializeOwned, Error as DeError};
use serde::{Deserialize, Deserializer};

/// Deserializes `T` from its proper JSON representation or a JSON-encoded
/// string containing it.
pub(crate) fn lenient<'de, T, D>(deserializer: D) -> Result<T, D::Error>
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
