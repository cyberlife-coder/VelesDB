//! Composite graph indexes for multi-property lookups (EPIC-047 US-001).
//!
//! Provides O(1) hash-based lookups on (label, property1, property2, ...) combinations.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

/// Index type for composite indexes.
// Reason: CompositeIndexType part of EPIC-047 composite graph index feature
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompositeIndexType {
    /// Hash index for equality lookups (O(1))
    Hash,
    /// Range index for range queries (O(log n))
    Range,
}

/// Composite index on (label, property1, property2, ...).
///
/// Provides O(1) lookups for nodes matching a label and specific property values.
/// Useful for queries like `MATCH (n:Person {name: 'Alice', city: 'Paris'})`.
// Reason: CompositeGraphIndex part of EPIC-047 composite graph index feature
#[allow(dead_code)]
#[derive(Debug, Serialize, Deserialize)]
pub struct CompositeGraphIndex {
    /// Label this index covers
    label: String,
    /// Property names in order
    properties: Vec<String>,
    /// Index type (hash or range)
    index_type: CompositeIndexType,
    /// (`property_values_hash`) -> Vec<NodeId>
    hash_index: HashMap<u64, Vec<u64>>,
}

// Reason: CompositeGraphIndex impl part of EPIC-047 composite graph index feature
#[allow(dead_code)]
impl CompositeGraphIndex {
    /// Creates a new composite index.
    #[must_use]
    pub fn new(
        label: impl Into<String>,
        properties: Vec<String>,
        index_type: CompositeIndexType,
    ) -> Self {
        Self {
            label: label.into(),
            properties,
            index_type,
            hash_index: HashMap::new(),
        }
    }

    /// Returns the label this index covers.
    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Returns the properties this index covers.
    #[must_use]
    pub fn properties(&self) -> &[String] {
        &self.properties
    }

    /// Returns the index type.
    #[must_use]
    pub fn index_type(&self) -> CompositeIndexType {
        self.index_type
    }

    /// Checks if this index covers the given label and properties.
    #[must_use]
    pub fn covers(&self, label: &str, properties: &[&str]) -> bool {
        if self.label != label {
            return false;
        }
        // Check if all requested properties are covered by this index
        properties
            .iter()
            .all(|p| self.properties.iter().any(|ip| ip == *p))
    }

    /// Inserts a node into the index.
    pub fn insert(&mut self, node_id: u64, values: &[Value]) {
        if values.len() != self.properties.len() {
            tracing::warn!(
                "CompositeGraphIndex: value count ({}) != property count ({})",
                values.len(),
                self.properties.len()
            );
            return;
        }

        let hash = Self::hash_values(values);
        self.hash_index.entry(hash).or_default().push(node_id);
    }

    /// Removes a node from the index.
    pub fn remove(&mut self, node_id: u64, values: &[Value]) -> bool {
        if values.len() != self.properties.len() {
            return false;
        }

        let hash = Self::hash_values(values);
        if let Some(nodes) = self.hash_index.get_mut(&hash) {
            if let Some(pos) = nodes.iter().position(|&id| id == node_id) {
                nodes.swap_remove(pos);
                if nodes.is_empty() {
                    self.hash_index.remove(&hash);
                }
                return true;
            }
        }
        false
    }

    /// Looks up nodes by property values.
    #[must_use]
    pub fn lookup(&self, values: &[Value]) -> &[u64] {
        if values.len() != self.properties.len() {
            return &[];
        }

        let hash = Self::hash_values(values);
        self.hash_index.get(&hash).map_or(&[], Vec::as_slice)
    }

    /// Returns the number of unique value combinations in the index.
    #[must_use]
    pub fn cardinality(&self) -> usize {
        self.hash_index.len()
    }

    /// Returns the total number of indexed nodes.
    #[must_use]
    pub fn node_count(&self) -> usize {
        self.hash_index.values().map(Vec::len).sum()
    }

    /// Clears the index.
    pub fn clear(&mut self) {
        self.hash_index.clear();
    }

    /// Computes a hash of the property values.
    fn hash_values(values: &[Value]) -> u64 {
        let mut hasher = DefaultHasher::new();
        for value in values {
            // Hash the JSON string representation for consistency
            value.to_string().hash(&mut hasher);
        }
        hasher.finish()
    }

    /// Gets the memory usage estimate in bytes.
    #[must_use]
    pub fn memory_usage(&self) -> usize {
        let mut total = std::mem::size_of::<Self>();
        total += self.label.len();
        for prop in &self.properties {
            total += prop.len();
        }
        // Each hash entry: u64 key + Vec overhead + node IDs
        for nodes in self.hash_index.values() {
            total += 8 + std::mem::size_of::<Vec<u64>>() + nodes.len() * 8;
        }
        total
    }
}

/// Manager for multiple composite indexes.
// Reason: CompositeIndexManager part of EPIC-047 composite graph index feature
#[allow(dead_code)]
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct CompositeIndexManager {
    /// All composite indexes, keyed by index name
    indexes: HashMap<String, CompositeGraphIndex>,
}

// Reason: CompositeIndexManager impl part of EPIC-047 composite graph index feature
#[allow(dead_code)]
impl CompositeIndexManager {
    /// Creates a new index manager.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a new composite index.
    pub fn create_index(
        &mut self,
        name: impl Into<String>,
        label: impl Into<String>,
        properties: Vec<String>,
        index_type: CompositeIndexType,
    ) -> bool {
        let name = name.into();
        if self.indexes.contains_key(&name) {
            return false;
        }
        let index = CompositeGraphIndex::new(label, properties, index_type);
        self.indexes.insert(name, index);
        true
    }

    /// Gets an index by name.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&CompositeGraphIndex> {
        self.indexes.get(name)
    }

    /// Gets a mutable index by name.
    pub fn get_mut(&mut self, name: &str) -> Option<&mut CompositeGraphIndex> {
        self.indexes.get_mut(name)
    }

    /// Drops an index by name.
    pub fn drop_index(&mut self, name: &str) -> bool {
        self.indexes.remove(name).is_some()
    }

    /// Finds indexes that cover the given label and properties.
    #[must_use]
    pub fn find_covering_indexes(&self, label: &str, properties: &[&str]) -> Vec<&str> {
        self.indexes
            .iter()
            .filter(|(_, idx)| idx.covers(label, properties))
            .map(|(name, _)| name.as_str())
            .collect()
    }

    /// Lists all index names.
    #[must_use]
    pub fn list_indexes(&self) -> Vec<&str> {
        self.indexes.keys().map(String::as_str).collect()
    }

    /// Updates all indexes when a node is added.
    pub fn on_add_node(&mut self, label: &str, node_id: u64, properties: &HashMap<String, Value>) {
        for index in self.indexes.values_mut() {
            if index.label() == label {
                // Extract values in the order of index properties
                let values: Vec<Value> = index
                    .properties()
                    .iter()
                    .map(|p| properties.get(p).cloned().unwrap_or(Value::Null))
                    .collect();
                index.insert(node_id, &values);
            }
        }
    }

    /// Updates all indexes when a node is removed.
    pub fn on_remove_node(
        &mut self,
        label: &str,
        node_id: u64,
        properties: &HashMap<String, Value>,
    ) {
        for index in self.indexes.values_mut() {
            if index.label() == label {
                let values: Vec<Value> = index
                    .properties()
                    .iter()
                    .map(|p| properties.get(p).cloned().unwrap_or(Value::Null))
                    .collect();
                index.remove(node_id, &values);
            }
        }
    }
}
