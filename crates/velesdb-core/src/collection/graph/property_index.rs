//! Property index for fast graph node lookups.
//!
//! Provides O(1) lookups on (label, property_name, value) instead of O(n) scans.

use roaring::RoaringBitmap;
use serde_json::Value;
use std::collections::HashMap;

/// Index for fast property-based node lookups.
///
/// Maps (label, property_name) -> (value -> node_ids) for O(1) lookups.
///
/// # Example
///
/// ```rust,ignore
/// let mut index = PropertyIndex::new();
/// index.create_index("Person", "email");
/// index.insert("Person", "email", &json!("alice@example.com"), 1);
///
/// let nodes = index.lookup("Person", "email", &json!("alice@example.com"));
/// assert!(nodes.map_or(false, |b| b.contains(1)));
/// ```
#[derive(Debug, Default)]
pub struct PropertyIndex {
    /// (label, property_name) -> (value_json -> node_ids)
    indexes: HashMap<(String, String), HashMap<String, RoaringBitmap>>,
}

impl PropertyIndex {
    /// Create a new empty property index.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Create an index for a (label, property) pair.
    ///
    /// This must be called before inserting values for this pair.
    pub fn create_index(&mut self, label: &str, property: &str) {
        let key = (label.to_string(), property.to_string());
        self.indexes.entry(key).or_default();
    }

    /// Check if an index exists for this (label, property) pair.
    #[must_use]
    pub fn has_index(&self, label: &str, property: &str) -> bool {
        let key = (label.to_string(), property.to_string());
        self.indexes.contains_key(&key)
    }

    /// Insert a node into the index.
    ///
    /// Returns `true` if the index exists and the node was added.
    pub fn insert(&mut self, label: &str, property: &str, value: &Value, node_id: u64) -> bool {
        let key = (label.to_string(), property.to_string());
        if let Some(value_map) = self.indexes.get_mut(&key) {
            let value_key = value.to_string();
            value_map
                .entry(value_key)
                .or_default()
                .insert(node_id as u32);
            true
        } else {
            false
        }
    }

    /// Remove a node from the index.
    ///
    /// Returns `true` if the node was removed.
    pub fn remove(&mut self, label: &str, property: &str, value: &Value, node_id: u64) -> bool {
        let key = (label.to_string(), property.to_string());
        if let Some(value_map) = self.indexes.get_mut(&key) {
            let value_key = value.to_string();
            if let Some(bitmap) = value_map.get_mut(&value_key) {
                let removed = bitmap.remove(node_id as u32);
                if bitmap.is_empty() {
                    value_map.remove(&value_key);
                }
                return removed;
            }
        }
        false
    }

    /// Lookup nodes by property value.
    ///
    /// Returns `None` if no index exists for this (label, property) pair.
    /// Returns `Some(&RoaringBitmap)` with matching node IDs (empty if no matches).
    #[must_use]
    pub fn lookup(&self, label: &str, property: &str, value: &Value) -> Option<&RoaringBitmap> {
        let key = (label.to_string(), property.to_string());
        self.indexes.get(&key).and_then(|value_map| {
            let value_key = value.to_string();
            value_map.get(&value_key)
        })
    }

    /// Get all indexed (label, property) pairs.
    #[must_use]
    pub fn indexed_properties(&self) -> Vec<(String, String)> {
        self.indexes.keys().cloned().collect()
    }

    /// Get the number of unique values for a (label, property) pair.
    #[must_use]
    pub fn cardinality(&self, label: &str, property: &str) -> Option<usize> {
        let key = (label.to_string(), property.to_string());
        self.indexes.get(&key).map(std::collections::HashMap::len)
    }

    /// Drop an index for a (label, property) pair.
    pub fn drop_index(&mut self, label: &str, property: &str) -> bool {
        let key = (label.to_string(), property.to_string());
        self.indexes.remove(&key).is_some()
    }

    /// Clear all indexes.
    pub fn clear(&mut self) {
        self.indexes.clear();
    }

    /// Get total memory estimate in bytes.
    #[must_use]
    pub fn memory_usage(&self) -> usize {
        let mut total = std::mem::size_of::<Self>();
        for ((label, prop), value_map) in &self.indexes {
            total += label.len() + prop.len();
            for (value_key, bitmap) in value_map {
                total += value_key.len();
                total += bitmap.serialized_size();
            }
        }
        total
    }
}
