//! Property index for fast graph node lookups.
//!
//! Provides O(1) lookups on (label, property_name, value) instead of O(n) scans.

use roaring::RoaringBitmap;
use serde::{Deserialize, Serialize};
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
#[derive(Debug, Default, Serialize, Deserialize)]
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
        self.indexes
            .keys()
            .any(|(l, p)| l == label && p == property)
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
        self.indexes
            .iter()
            .find(|((l, p), _)| l == label && p == property)
            .and_then(|(_, value_map)| {
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
        self.indexes
            .iter()
            .find(|((l, p), _)| l == label && p == property)
            .map(|(_, value_map)| value_map.len())
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

    // =========================================================================
    // Maintenance hooks - called automatically on graph mutations
    // =========================================================================

    /// Hook called when a node is added to the graph.
    ///
    /// Indexes all properties that have an active index.
    pub fn on_add_node(&mut self, label: &str, node_id: u64, properties: &HashMap<String, Value>) {
        for (prop_name, value) in properties {
            if self.has_index(label, prop_name) {
                self.insert(label, prop_name, value, node_id);
            }
        }
    }

    /// Hook called when a node is removed from the graph.
    ///
    /// Removes all indexed properties for this node.
    pub fn on_remove_node(
        &mut self,
        label: &str,
        node_id: u64,
        properties: &HashMap<String, Value>,
    ) {
        for (prop_name, value) in properties {
            if self.has_index(label, prop_name) {
                self.remove(label, prop_name, value, node_id);
            }
        }
    }

    /// Hook called when a property is updated on a node.
    ///
    /// Removes old value and inserts new value if property is indexed.
    pub fn on_update_property(
        &mut self,
        label: &str,
        node_id: u64,
        property: &str,
        old_value: &Value,
        new_value: &Value,
    ) {
        if self.has_index(label, property) {
            self.remove(label, property, old_value, node_id);
            self.insert(label, property, new_value, node_id);
        }
    }

    /// Hook to index all properties of a node after creating an index.
    ///
    /// Use this to backfill an index after creation.
    pub fn index_node(&mut self, label: &str, node_id: u64, properties: &HashMap<String, Value>) {
        self.on_add_node(label, node_id, properties);
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
