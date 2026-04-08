//! Serde helpers for serializing `u64` IDs as JSON strings.
//!
//! JavaScript `Number.MAX_SAFE_INTEGER` is 2^53 - 1. Any `u64` above that
//! threshold loses precision when parsed via `JSON.parse()`. These helpers
//! serialize IDs as quoted strings (`"12345"`) while accepting both strings
//! and numbers on deserialization for backward compatibility.
//!
//! # Usage
//!
//! ```ignore
//! use serde::{Deserialize, Serialize};
//! use crate::api_types::serde_id;
//!
//! #[derive(Serialize, Deserialize)]
//! struct Response {
//!     #[serde(
//!         serialize_with = "serde_id::serialize_id_as_string",
//!         deserialize_with = "serde_id::deserialize_id_from_string_or_number"
//!     )]
//!     id: u64,
//! }
//! ```

use serde::de::{self, Unexpected, Visitor};
use serde::{Deserializer, Serializer};
use std::fmt;

/// Serializes a `u64` as a JSON string to prevent JavaScript precision loss.
///
/// Emits `"12345"` instead of `12345`.
///
/// # Errors
///
/// Returns `S::Error` if the serializer rejects the string value.
pub fn serialize_id_as_string<S: Serializer>(
    value: &u64,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&value.to_string())
}

/// Deserializes a `u64` from either a JSON string or a JSON number.
///
/// Accepts both `"12345"` (string) and `12345` (number) for backward
/// compatibility with clients that have not yet migrated to string IDs.
///
/// # Errors
///
/// Returns `D::Error` if the input is neither a valid u64 string nor a
/// non-negative integer.
pub fn deserialize_id_from_string_or_number<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<u64, D::Error> {
    deserializer.deserialize_any(IdVisitor)
}

/// Visitor that accepts either a JSON string or a JSON number as a `u64`.
struct IdVisitor;

impl Visitor<'_> for IdVisitor {
    type Value = u64;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a u64 as a string or number")
    }

    fn visit_u64<E: de::Error>(self, value: u64) -> Result<Self::Value, E> {
        Ok(value)
    }

    fn visit_i64<E: de::Error>(self, value: i64) -> Result<Self::Value, E> {
        u64::try_from(value).map_err(|_| {
            de::Error::invalid_value(Unexpected::Signed(value), &"a non-negative integer")
        })
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
        value
            .parse::<u64>()
            .map_err(|_| de::Error::invalid_value(Unexpected::Str(value), &"a u64 string"))
    }
}
