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
use serde::{Deserialize, Deserializer, Serializer};
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

/// `OpenAPI` schema for input `id` fields that accept either a JSON integer
/// (native form) or a string. Mirrors [`deserialize_id_from_string_or_number`]:
/// clients may send `12345` or `"12345"`, the latter being precision-safe for
/// `u64` values above 2^53-1.
///
/// Apply with
/// `#[cfg_attr(feature = "openapi", schema(schema_with = serde_id::id_input_schema))]`.
#[cfg(feature = "openapi")]
#[must_use]
pub fn id_input_schema() -> utoipa::openapi::schema::OneOfBuilder {
    id_oneof().description(Some(
        "Point ID. Accepts a JSON integer (native form) or a string; use a \
         string for u64 values above 2^53-1 to avoid JavaScript precision loss.",
    ))
}

/// Shared `int | string` oneOf used by all ID schema helpers.
///
/// An ID is emitted as a string on the wire (precision-safe) but accepted as
/// either an integer or a string on input.
///
/// `u64` has no exact `OpenAPI` primitive, so the two branches are deliberately
/// asymmetric and the exact bound is enforced server-side (`parse::<u64>`):
/// the integer branch is `int64`/`minimum: 0` (the JS-safe native form; values
/// above `i64::MAX` must use the string branch, which is precisely why the
/// string form exists), and the string branch is `^[0-9]+$` — permissive at
/// the very top of the range (a >`u64::MAX` digit string is rejected at
/// runtime) rather than an unreadable exact-max regex. This mirrors the
/// protobuf/Google-JSON convention of carrying 64-bit ints as strings.
#[cfg(feature = "openapi")]
pub(crate) fn id_oneof() -> utoipa::openapi::schema::OneOfBuilder {
    use utoipa::openapi::schema::{KnownFormat, ObjectBuilder, OneOfBuilder, SchemaFormat, Type};
    OneOfBuilder::new()
        .item(
            ObjectBuilder::new()
                .schema_type(Type::Integer)
                .format(Some(SchemaFormat::KnownFormat(KnownFormat::Int64)))
                // IDs are `u64`; the native-integer branch is non-negative.
                .minimum(Some(0.0)),
        )
        .item(
            ObjectBuilder::new()
                .schema_type(Type::String)
                // The string branch carries the decimal digits of a `u64`.
                .pattern(Some("^[0-9]+$")),
        )
}

/// `OpenAPI` schema for an array of IDs whose elements accept an integer or a
/// string (precision-safe), mirroring [`serialize_ids_as_strings`].
///
/// Apply with
/// `#[cfg_attr(feature = "openapi", schema(schema_with = serde_id::ids_array_schema))]`.
#[cfg(feature = "openapi")]
#[must_use]
pub fn ids_array_schema() -> utoipa::openapi::schema::ArrayBuilder {
    utoipa::openapi::schema::ArrayBuilder::new()
        .items(id_oneof())
        .description(Some(
            "Point IDs. Each accepts a JSON integer (native form) or a string; \
             use a string for u64 values above 2^53-1 to avoid JavaScript \
             precision loss.",
        ))
}

/// `OpenAPI` schema for a map whose values are IDs accepting an integer or a
/// string (precision-safe), mirroring [`serialize_id_map_as_strings`].
///
/// Apply with
/// `#[cfg_attr(feature = "openapi", schema(schema_with = serde_id::id_map_schema))]`.
#[cfg(feature = "openapi")]
#[must_use]
pub fn id_map_schema() -> utoipa::openapi::schema::ObjectBuilder {
    use utoipa::openapi::schema::Schema;
    utoipa::openapi::schema::ObjectBuilder::new()
        .additional_properties(Some(Schema::OneOf(id_oneof().build())))
}

/// Serializes a slice of `u64` IDs as an array of JSON strings.
///
/// Emits `["1","2"]` instead of `[1,2]` to keep IDs above 2^53-1 precise in
/// JavaScript. Usable as `serialize_with` for a `Vec<u64>` field.
///
/// # Errors
///
/// Returns `S::Error` if the serializer rejects the sequence.
pub fn serialize_ids_as_strings<S: Serializer>(
    values: &[u64],
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.collect_seq(values.iter().map(u64::to_string))
}

/// Serializes a `HashMap<String, u64>` as a JSON object whose values are
/// strings, keeping IDs above 2^53-1 precise in JavaScript.
///
/// Emits `{"k":"1"}` instead of `{"k":1}`.
///
/// # Errors
///
/// Returns `S::Error` if the serializer rejects the map.
#[allow(clippy::implicit_hasher)] // applied to a concrete `HashMap<String, u64>` field
pub fn serialize_id_map_as_strings<S: Serializer>(
    map: &std::collections::HashMap<String, u64>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.collect_map(map.iter().map(|(k, v)| (k, v.to_string())))
}

/// Deserializes a `Vec<u64>` whose elements may be JSON strings or numbers.
///
/// Accepts `["1","2"]` and `[1,2]` for backward compatibility.
///
/// # Errors
///
/// Returns `D::Error` if any element is not a valid u64 string or number.
pub fn deserialize_ids_from_string_or_number<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<u64>, D::Error> {
    let fields = Vec::<IdField>::deserialize(deserializer)?;
    Ok(fields.into_iter().map(|f| f.0).collect())
}

/// Deserializes a `HashMap<String, u64>` whose values may be JSON strings or
/// numbers.
///
/// Accepts `{"k":"1"}` and `{"k":1}` for backward compatibility.
///
/// # Errors
///
/// Returns `D::Error` if any value is not a valid u64 string or number.
pub fn deserialize_id_map_from_string_or_number<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<std::collections::HashMap<String, u64>, D::Error> {
    let fields = std::collections::HashMap::<String, IdField>::deserialize(deserializer)?;
    Ok(fields.into_iter().map(|(k, f)| (k, f.0)).collect())
}

/// Newtype reusing [`deserialize_id_from_string_or_number`] so collection
/// deserializers (`Vec`, `HashMap`) stay trivial and complexity-safe.
struct IdField(u64);

impl<'de> serde::Deserialize<'de> for IdField {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserialize_id_from_string_or_number(deserializer).map(IdField)
    }
}

/// Serializes an `Option<u64>` as a JSON string when `Some`, or `null` when `None`.
///
/// Emits `"12345"` for `Some(12345)` and `null` for `None`.
///
/// # Errors
///
/// Returns `S::Error` if the serializer rejects the value.
pub fn serialize_option_id_as_string<S: Serializer>(
    value: &Option<u64>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match value {
        Some(id) => serializer.serialize_str(&id.to_string()),
        None => serializer.serialize_none(),
    }
}

/// Deserializes an `Option<u64>` from a JSON string, number, or null.
///
/// Accepts `"12345"` (string), `12345` (number), and `null` for backward
/// compatibility with clients that have not yet migrated to string IDs.
///
/// # Errors
///
/// Returns `D::Error` if the input is present but not a valid u64 string
/// or non-negative integer.
pub fn deserialize_option_id_from_string_or_number<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<u64>, D::Error> {
    deserializer.deserialize_option(OptionIdVisitor)
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

/// Visitor for `Option<u64>` that delegates to `IdVisitor` for `Some` values.
struct OptionIdVisitor;

impl<'de> Visitor<'de> for OptionIdVisitor {
    type Value = Option<u64>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("null, a u64 string, or a u64 number")
    }

    fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
        Ok(None)
    }

    fn visit_some<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_any(IdVisitor).map(Some)
    }

    fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
        Ok(None)
    }
}
