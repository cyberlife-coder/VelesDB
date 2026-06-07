//! Secondary index types for metadata payload fields.

#[cfg(test)]
mod bitmap_tests;

use parking_lot::RwLock;
use serde_json::Number;
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::ops::Bound;

/// Orderable JSON primitive value used as a key in secondary indexes.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum JsonValue {
    /// String JSON value.
    String(String),
    /// Numeric JSON value (normalized to f64 bits).
    Number(F64Key),
    /// Boolean JSON value.
    Bool(bool),
}

/// Wrapper type that provides total ordering for f64 values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct F64Key(u64);

impl From<f64> for F64Key {
    fn from(value: f64) -> Self {
        Self(value.to_bits())
    }
}

impl PartialOrd for F64Key {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for F64Key {
    fn cmp(&self, other: &Self) -> Ordering {
        f64::from_bits(self.0).total_cmp(&f64::from_bits(other.0))
    }
}

impl PartialOrd for JsonValue {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for JsonValue {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::Bool(a), Self::Bool(b)) => a.cmp(b),
            (Self::Number(a), Self::Number(b)) => a.cmp(b),
            (Self::String(a), Self::String(b)) => a.cmp(b),
            (Self::Bool(_), _) | (Self::Number(_), Self::String(_)) => Ordering::Less,
            (Self::Number(_), Self::Bool(_)) | (Self::String(_), _) => Ordering::Greater,
        }
    }
}

impl JsonValue {
    /// Converts JSON payload primitive to an orderable key.
    #[must_use]
    pub fn from_json(value: &serde_json::Value) -> Option<Self> {
        match value {
            serde_json::Value::String(s) => Some(Self::String(s.clone())),
            serde_json::Value::Number(n) => Self::number_from_json(n),
            serde_json::Value::Bool(b) => Some(Self::Bool(*b)),
            _ => None,
        }
    }

    /// Converts `VelesQL` AST value into an index key.
    #[must_use]
    pub fn from_ast_value(value: &crate::velesql::Value) -> Option<Self> {
        match value {
            crate::velesql::Value::String(s) => Some(Self::String(s.clone())),
            #[allow(clippy::cast_precision_loss)]
            // Reason: index keys normalize all numerics to f64 for ordering.
            crate::velesql::Value::Integer(i) => Some(Self::Number(F64Key::from(*i as f64))),
            #[allow(clippy::cast_precision_loss)]
            // Reason: index keys normalize all numerics to f64 for ordering.
            crate::velesql::Value::UnsignedInteger(u) => {
                Some(Self::Number(F64Key::from(*u as f64)))
            }
            crate::velesql::Value::Float(f) => Some(Self::Number(F64Key::from(*f))),
            crate::velesql::Value::Boolean(b) => Some(Self::Bool(*b)),
            _ => None,
        }
    }

    fn number_from_json(number: &Number) -> Option<Self> {
        number.as_f64().map(|v| Self::Number(F64Key::from(v)))
    }
}

/// Secondary index implementation.
#[derive(Debug)]
pub(crate) enum SecondaryIndex {
    /// B-tree index mapping JSON primitive values to point IDs.
    BTree(RwLock<BTreeMap<JsonValue, Vec<u64>>>),
}

impl SecondaryIndex {
    /// Returns a [`RoaringBitmap`] of all point IDs matching the given value.
    ///
    /// The bitmap is built on-the-fly from the B-tree leaf. Returns
    /// `Some(empty)` when the value has no entries (a valid "no matches"
    /// pre-filter). Returns `None` when any matching ID exceeds [`u32::MAX`] and
    /// therefore cannot be represented in the bitmap — signalling an
    /// **incomplete** result so callers fall back to a full scan rather than
    /// silently dropping the high ID (correctness over the optimization).
    #[must_use]
    pub fn to_bitmap(&self, value: &JsonValue) -> Option<roaring::RoaringBitmap> {
        match self {
            Self::BTree(tree) => {
                let guard = tree.read();
                match guard.get(value) {
                    Some(ids) => ids_to_bitmap(ids),
                    None => Some(roaring::RoaringBitmap::new()),
                }
            }
        }
    }

    /// Returns a [`RoaringBitmap`] of all point IDs whose key falls within
    /// the given range bounds.
    ///
    /// Uses `BTreeMap::range()` for efficient ordered iteration. This powers
    /// Gt, Gte, Lt, Lte, and BETWEEN pre-filters. Returns `Some(empty)` when no
    /// keys fall within the range, and `None` when any in-range ID exceeds
    /// [`u32::MAX`] (incomplete — callers must fall back to a full scan).
    #[must_use]
    pub fn range_bitmap(
        &self,
        from: Bound<&JsonValue>,
        to: Bound<&JsonValue>,
    ) -> Option<roaring::RoaringBitmap> {
        match self {
            Self::BTree(tree) => {
                let guard = tree.read();
                let mut bm = roaring::RoaringBitmap::new();
                for ids in guard.range((from, to)).map(|(_, v)| v) {
                    for &id in ids {
                        bm.insert(u32::try_from(id).ok()?);
                    }
                }
                Some(bm)
            }
        }
    }
}

/// Converts a slice of `u64` point IDs into a [`RoaringBitmap`].
///
/// `RoaringBitmap` stores `u32` values. Returns `None` if any ID exceeds
/// [`u32::MAX`]: the bitmap would silently omit that ID, so callers that fetch
/// only the bitmap's IDs (e.g. the JOIN pre-filter) would drop a real match.
/// Signalling `None` forces those callers to fall back to a full scan.
fn ids_to_bitmap(ids: &[u64]) -> Option<roaring::RoaringBitmap> {
    let mut bm = roaring::RoaringBitmap::new();
    for &id in ids {
        bm.insert(u32::try_from(id).ok()?);
    }
    Some(bm)
}
