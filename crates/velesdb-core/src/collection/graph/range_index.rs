//! Range index for ordered property lookups using BTreeMap.
//!
//! Provides O(log n) range queries (>, <, >=, <=, BETWEEN) instead of O(n) scans.

// SAFETY: Numeric casts in range index are intentional:
// - i64->f64 for mixed-type comparisons: precision loss acceptable for ordering
// - Values represent property values, exact precision not required for comparisons
#![allow(clippy::cast_precision_loss)]

use roaring::RoaringBitmap;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::ops::Bound;

/// Wrapper for comparable numeric values in BTreeMap.
///
/// JSON values are converted to this for ordered comparison.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderedValue {
    /// Null value (sorts first)
    Null,
    /// Integer value
    Integer(i64),
    /// Float value
    Float(OrderedFloat),
    /// String value (lexicographic order)
    String(String),
}

/// Wrapper for f64 that implements Ord (NaN sorts last).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct OrderedFloat(pub f64);

impl PartialOrd for OrderedFloat {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Eq for OrderedFloat {}

impl Ord for OrderedFloat {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.partial_cmp(&other.0).unwrap_or_else(|| {
            // Handle NaN: NaN sorts after everything
            match (self.0.is_nan(), other.0.is_nan()) {
                (true, true) => std::cmp::Ordering::Equal,
                (true, false) => std::cmp::Ordering::Greater,
                (false, true) => std::cmp::Ordering::Less,
                (false, false) => unreachable!(),
            }
        })
    }
}

impl PartialOrd for OrderedValue {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for OrderedValue {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match (self, other) {
            (Self::Null, Self::Null) => std::cmp::Ordering::Equal,
            (Self::Null, _) | (Self::Integer(_) | Self::Float(_), Self::String(_)) => {
                std::cmp::Ordering::Less
            }
            (_, Self::Null) | (Self::String(_), Self::Integer(_) | Self::Float(_)) => {
                std::cmp::Ordering::Greater
            }
            (Self::Integer(a), Self::Integer(b)) => a.cmp(b),
            (Self::Float(a), Self::Float(b)) => a.cmp(b),
            (Self::Integer(a), Self::Float(b)) => OrderedFloat(*a as f64).cmp(b),
            (Self::Float(a), Self::Integer(b)) => a.cmp(&OrderedFloat(*b as f64)),
            (Self::String(a), Self::String(b)) => a.cmp(b),
        }
    }
}

impl OrderedValue {
    /// Convert a JSON value to an OrderedValue for comparison.
    #[must_use]
    pub fn from_json(value: &serde_json::Value) -> Option<Self> {
        match value {
            serde_json::Value::Null => Some(Self::Null),
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    Some(Self::Integer(i))
                } else {
                    n.as_f64().map(|f| Self::Float(OrderedFloat(f)))
                }
            }
            serde_json::Value::String(s) => Some(Self::String(s.clone())),
            _ => None, // Arrays and objects are not comparable
        }
    }
}

/// Current schema version for RangeIndex serialization.
/// Increment this when making breaking changes to the index format.
pub const RANGE_INDEX_VERSION: u32 = 1;

/// Range index for ordered property lookups.
///
/// Uses BTreeMap for O(log n) range queries on numeric/string properties.
///
/// # Example
///
/// ```rust,ignore
/// let mut index = RangeIndex::new();
/// index.create_index("Event", "timestamp");
/// index.insert("Event", "timestamp", &json!(1704067200), 1);
/// index.insert("Event", "timestamp", &json!(1704153600), 2);
///
/// // Range query: timestamp > 1704067200
/// let nodes = index.range_greater_than("Event", "timestamp", &json!(1704067200));
/// ```
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct RangeIndex {
    /// Schema version for forward compatibility.
    #[serde(default = "default_range_version")]
    version: u32,
    /// (label, property_name) -> (ordered_value -> node_ids)
    indexes: HashMap<(String, String), BTreeMap<OrderedValue, RoaringBitmap>>,
}

fn default_range_version() -> u32 {
    RANGE_INDEX_VERSION
}

impl RangeIndex {
    /// Create a new empty range index.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a range index for a (label, property) pair.
    pub fn create_index(&mut self, label: &str, property: &str) {
        let key = (label.to_string(), property.to_string());
        self.indexes.entry(key).or_default();
    }

    /// Check if a range index exists for this (label, property) pair.
    #[must_use]
    pub fn has_index(&self, label: &str, property: &str) -> bool {
        self.indexes
            .keys()
            .any(|(l, p)| l == label && p == property)
    }

    /// Insert a node into the range index.
    ///
    /// Returns `true` if the index exists and the value is comparable.
    /// Returns `false` if node_id > u32::MAX to prevent data corruption.
    pub fn insert(
        &mut self,
        label: &str,
        property: &str,
        value: &serde_json::Value,
        node_id: u64,
    ) -> bool {
        // BUG FIX: Reject node_id > u32::MAX instead of silently truncating
        let Some(safe_id) = u32::try_from(node_id).ok() else {
            return false;
        };

        let key = (label.to_string(), property.to_string());
        if let Some(btree) = self.indexes.get_mut(&key) {
            if let Some(ordered) = OrderedValue::from_json(value) {
                btree.entry(ordered).or_default().insert(safe_id);
                return true;
            }
        }
        false
    }

    /// Remove a node from the range index.
    ///
    /// Returns `false` if node_id > u32::MAX (cannot exist in index).
    pub fn remove(
        &mut self,
        label: &str,
        property: &str,
        value: &serde_json::Value,
        node_id: u64,
    ) -> bool {
        // BUG FIX: node_id > u32::MAX cannot exist in index
        let Some(safe_id) = u32::try_from(node_id).ok() else {
            return false;
        };

        let key = (label.to_string(), property.to_string());
        if let Some(btree) = self.indexes.get_mut(&key) {
            if let Some(ordered) = OrderedValue::from_json(value) {
                if let Some(bitmap) = btree.get_mut(&ordered) {
                    let removed = bitmap.remove(safe_id);
                    if bitmap.is_empty() {
                        btree.remove(&ordered);
                    }
                    return removed;
                }
            }
        }
        false
    }

    /// Range query: value > threshold
    #[must_use]
    pub fn range_greater_than(
        &self,
        label: &str,
        property: &str,
        threshold: &serde_json::Value,
    ) -> RoaringBitmap {
        self.range_query(
            label,
            property,
            Bound::Excluded(threshold),
            Bound::Unbounded,
        )
    }

    /// Range query: value >= threshold
    #[must_use]
    pub fn range_greater_or_equal(
        &self,
        label: &str,
        property: &str,
        threshold: &serde_json::Value,
    ) -> RoaringBitmap {
        self.range_query(
            label,
            property,
            Bound::Included(threshold),
            Bound::Unbounded,
        )
    }

    /// Range query: value < threshold
    #[must_use]
    pub fn range_less_than(
        &self,
        label: &str,
        property: &str,
        threshold: &serde_json::Value,
    ) -> RoaringBitmap {
        self.range_query(
            label,
            property,
            Bound::Unbounded,
            Bound::Excluded(threshold),
        )
    }

    /// Range query: value <= threshold
    #[must_use]
    pub fn range_less_or_equal(
        &self,
        label: &str,
        property: &str,
        threshold: &serde_json::Value,
    ) -> RoaringBitmap {
        self.range_query(
            label,
            property,
            Bound::Unbounded,
            Bound::Included(threshold),
        )
    }

    /// Range query: start <= value <= end (BETWEEN)
    #[must_use]
    pub fn range_between(
        &self,
        label: &str,
        property: &str,
        start: &serde_json::Value,
        end: &serde_json::Value,
    ) -> RoaringBitmap {
        self.range_query(
            label,
            property,
            Bound::Included(start),
            Bound::Included(end),
        )
    }

    /// Internal range query using BTreeMap::range()
    fn range_query(
        &self,
        label: &str,
        property: &str,
        start: Bound<&serde_json::Value>,
        end: Bound<&serde_json::Value>,
    ) -> RoaringBitmap {
        let mut result = RoaringBitmap::new();

        let key = (label.to_string(), property.to_string());
        let Some(btree) = self.indexes.get(&key) else {
            return result;
        };

        // Convert bounds to OrderedValue
        let start_bound = match start {
            Bound::Included(v) => OrderedValue::from_json(v).map(Bound::Included),
            Bound::Excluded(v) => OrderedValue::from_json(v).map(Bound::Excluded),
            Bound::Unbounded => Some(Bound::Unbounded),
        };

        let end_bound = match end {
            Bound::Included(v) => OrderedValue::from_json(v).map(Bound::Included),
            Bound::Excluded(v) => OrderedValue::from_json(v).map(Bound::Excluded),
            Bound::Unbounded => Some(Bound::Unbounded),
        };

        let (Some(start_ord), Some(end_ord)) = (start_bound, end_bound) else {
            return result;
        };

        // Use BTreeMap::range() for O(log n) access
        for bitmap in btree.range((start_ord, end_ord)).map(|(_k, v)| v) {
            result |= bitmap;
        }

        result
    }

    /// Drop a range index.
    pub fn drop_index(&mut self, label: &str, property: &str) -> bool {
        let key = (label.to_string(), property.to_string());
        self.indexes.remove(&key).is_some()
    }

    /// Clear all range indexes.
    pub fn clear(&mut self) {
        self.indexes.clear();
    }

    /// Get memory usage estimate.
    #[must_use]
    pub fn memory_usage(&self) -> usize {
        let mut total = std::mem::size_of::<Self>();
        for ((label, prop), btree) in &self.indexes {
            total += label.len() + prop.len();
            for bitmap in btree.values() {
                total += std::mem::size_of::<OrderedValue>();
                total += bitmap.serialized_size();
            }
        }
        total
    }

    /// Get all indexed (label, property) pairs.
    #[must_use]
    pub fn indexed_properties(&self) -> Vec<(String, String)> {
        self.indexes.keys().cloned().collect()
    }

    // =========================================================================
    // Persistence - serialize/deserialize index to/from bytes
    // =========================================================================

    /// Serialize the index to bytes using bincode.
    ///
    /// # Errors
    /// Returns an error if serialization fails.
    pub fn to_bytes(&self) -> Result<Vec<u8>, bincode::Error> {
        bincode::serialize(self)
    }

    /// Deserialize an index from bytes.
    ///
    /// # Errors
    /// Returns an error if deserialization fails (corrupted data).
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, bincode::Error> {
        bincode::deserialize(bytes)
    }

    /// Save the index to a file.
    ///
    /// # Errors
    /// Returns an error if serialization or file I/O fails.
    pub fn save_to_file(&self, path: &std::path::Path) -> std::io::Result<()> {
        let bytes = self
            .to_bytes()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
        std::fs::write(path, bytes)
    }

    /// Load an index from a file.
    ///
    /// # Errors
    /// Returns an error if file I/O or deserialization fails.
    pub fn load_from_file(path: &std::path::Path) -> std::io::Result<Self> {
        let bytes = std::fs::read(path)?;
        Self::from_bytes(&bytes)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))
    }
}
