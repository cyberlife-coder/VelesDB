//! One representation per group key, so rows equal by value share a group.
//!
//! A [`Grouper`](super::grouping::Grouper) keys a row by its GROUP BY values.
//! Numbers are one key by exact value, the equality of
//! [`compare_json_values`](super::compare_json_values); every other value —
//! strings, booleans, arrays, objects — is one key by JSON equality, numbers
//! nested in an array or object included, which are not normalised.

use std::borrow::Cow;
use std::hash::Hash;

use rustc_hash::FxHasher;

/// A number as a group key: an integer — an integral float that an `i64` or
/// `u64` holds included (`1.0`, `-0.0`, `2^53`) — or any other float by its
/// bits. Two numbers are one key exactly when
/// [`compare_json_values`](super::compare_json_values) finds
/// them equal: no other float equals an integer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum NumberKey {
    Integer(i128),
    Float(u64),
}

/// 2^64: an integral float below it, and at least -2^63, is an `i64` or a
/// `u64`.
const U64_RANGE_END: f64 = 18_446_744_073_709_551_616.0;
const I64_RANGE_START: f64 = -9_223_372_036_854_775_808.0;

impl NumberKey {
    fn of(n: &serde_json::Number) -> Self {
        if let Some(integer) = super::json_order::integer(n) {
            return Self::Integer(integer);
        }
        let f = n.as_f64().unwrap_or(0.0);
        let integral = matches!(f.fract().classify(), std::num::FpCategory::Zero);
        if integral && (I64_RANGE_START..U64_RANGE_END).contains(&f) {
            // Reason: `f` is integral and within the i64/u64 range: exact.
            #[allow(clippy::cast_possible_truncation)]
            Self::Integer(f as i128)
        } else {
            Self::Float(f.to_bits())
        }
    }

    /// The number this key stands for, one representation per key.
    fn to_json(self, original: &serde_json::Number) -> serde_json::Value {
        match self {
            Self::Integer(i) => i64::try_from(i)
                .map(serde_json::Value::from)
                .or_else(|_| u64::try_from(i).map(serde_json::Value::from))
                .unwrap_or_else(|_| serde_json::Value::Number(original.clone())),
            Self::Float(_) => serde_json::Value::Number(original.clone()),
        }
    }
}

/// A group key value with one representation per number ([`NumberKey`]):
/// `1.0` is stored as `1`, `-0.0` as `0`. Every other value is kept.
pub(super) fn normalise(value: Cow<'_, serde_json::Value>) -> serde_json::Value {
    match value.as_ref() {
        serde_json::Value::Number(n) => NumberKey::of(n).to_json(n),
        _ => value.into_owned(),
    }
}

/// Whether a row's key value is the stored (normalised) one.
pub(super) fn same_key(stored: &serde_json::Value, value: Option<&serde_json::Value>) -> bool {
    match (stored, value) {
        (serde_json::Value::Null, None) => true,
        (serde_json::Value::Number(a), Some(serde_json::Value::Number(b))) => {
            NumberKey::of(a) == NumberKey::of(b)
        }
        (stored, Some(value)) => stored == value,
        (_, None) => false,
    }
}

/// Hashes a row's key value, a missing one as `null`, numbers by
/// [`NumberKey`], so values [`same_key`] finds equal hash alike.
pub(super) fn hash_key_value(value: Option<&serde_json::Value>, hasher: &mut FxHasher) {
    match value.unwrap_or(&serde_json::Value::Null) {
        serde_json::Value::Null => 0u8.hash(hasher),
        serde_json::Value::Bool(b) => {
            1u8.hash(hasher);
            b.hash(hasher);
        }
        serde_json::Value::Number(n) => {
            2u8.hash(hasher);
            NumberKey::of(n).hash(hasher);
        }
        serde_json::Value::String(s) => {
            3u8.hash(hasher);
            s.hash(hasher);
        }
        other => {
            4u8.hash(hasher);
            other.to_string().hash(hasher);
        }
    }
}
