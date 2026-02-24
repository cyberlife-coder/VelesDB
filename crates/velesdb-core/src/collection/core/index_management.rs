//! Index management methods for Collection (EPIC-009 propagation).

use crate::collection::types::Collection;
use crate::error::Result;
use crate::index::{JsonValue, SecondaryIndex};
use parking_lot::RwLock;
use std::collections::BTreeMap;

/// Index information response for API.
#[derive(Debug, Clone)]
pub struct IndexInfo {
    /// Node label.
    pub label: String,
    /// Property name.
    pub property: String,
    /// Index type (hash or range).
    pub index_type: String,
    /// Number of unique values indexed.
    pub cardinality: usize,
    /// Memory usage in bytes.
    pub memory_bytes: usize,
}

impl Collection {
    /// Creates a secondary metadata index for a payload field.
    ///
    /// # Errors
    ///
    /// Returns Ok(()) on success. Index creation is idempotent.
    pub fn create_index(&self, field_name: &str) -> Result<()> {
        let mut indexes = self.secondary_indexes.write();
        indexes
            .entry(field_name.to_string())
            .or_insert_with(|| SecondaryIndex::BTree(RwLock::new(BTreeMap::new())));
        Ok(())
    }

    /// Checks whether a secondary metadata index exists for a field.
    #[must_use]
    pub fn has_secondary_index(&self, field_name: &str) -> bool {
        self.secondary_indexes.read().contains_key(field_name)
    }

    /// Looks up matching point IDs for an indexed field value.
    #[must_use]
    pub fn secondary_index_lookup(&self, field_name: &str, value: &JsonValue) -> Option<Vec<u64>> {
        let indexes = self.secondary_indexes.read();
        let index = indexes.get(field_name)?;
        match index {
            SecondaryIndex::BTree(tree) => tree.read().get(value).cloned(),
        }
    }

    /// Create a property index for O(1) equality lookups.
    ///
    /// # Arguments
    ///
    /// * `label` - Node label to index (e.g., "Person")
    /// * `property` - Property name to index (e.g., "email")
    ///
    /// # Errors
    ///
    /// Returns Ok(()) on success. Index creation is idempotent.
    pub fn create_property_index(&self, label: &str, property: &str) -> Result<()> {
        let mut index = self.property_index.write();
        index.create_index(label, property);
        Ok(())
    }

    /// Create a range index for O(log n) range queries.
    ///
    /// # Arguments
    ///
    /// * `label` - Node label to index (e.g., "Event")
    /// * `property` - Property name to index (e.g., "timestamp")
    ///
    /// # Errors
    ///
    /// Returns Ok(()) on success. Index creation is idempotent.
    pub fn create_range_index(&self, label: &str, property: &str) -> Result<()> {
        let mut index = self.range_index.write();
        index.create_index(label, property);
        Ok(())
    }

    /// Check if a property index exists.
    #[must_use]
    pub fn has_property_index(&self, label: &str, property: &str) -> bool {
        self.property_index.read().has_index(label, property)
    }

    /// Check if a range index exists.
    #[must_use]
    pub fn has_range_index(&self, label: &str, property: &str) -> bool {
        self.range_index.read().has_index(label, property)
    }

    /// List all indexes on this collection.
    #[must_use]
    pub fn list_indexes(&self) -> Vec<IndexInfo> {
        let mut indexes = Vec::new();

        // List property (hash) indexes
        let prop_index = self.property_index.read();
        for (label, property) in prop_index.indexed_properties() {
            let cardinality = prop_index.cardinality(&label, &property).unwrap_or(0);
            indexes.push(IndexInfo {
                label,
                property,
                index_type: "hash".to_string(),
                cardinality,
                memory_bytes: 0, // Approximation
            });
        }

        // List range indexes
        let range_idx = self.range_index.read();
        for (label, property) in range_idx.indexed_properties() {
            indexes.push(IndexInfo {
                label,
                property,
                index_type: "range".to_string(),
                cardinality: 0, // Range indexes don't track cardinality the same way
                memory_bytes: 0,
            });
        }

        indexes
    }

    /// Drop an index (either property or range).
    ///
    /// # Arguments
    ///
    /// * `label` - Node label
    /// * `property` - Property name
    ///
    /// # Returns
    ///
    /// Ok(true) if an index was dropped, Ok(false) if no index existed.
    ///
    /// # Errors
    ///
    /// Returns an error if underlying index stores fail while dropping.
    pub fn drop_index(&self, label: &str, property: &str) -> Result<bool> {
        // Try property index first
        let dropped_prop = self.property_index.write().drop_index(label, property);
        if dropped_prop {
            return Ok(true);
        }

        // Try range index
        let dropped_range = self.range_index.write().drop_index(label, property);
        Ok(dropped_range)
    }

    /// Get total memory usage of all indexes.
    #[must_use]
    pub fn indexes_memory_usage(&self) -> usize {
        let prop_mem = self.property_index.read().memory_usage();
        let range_mem = self.range_index.read().memory_usage();
        prop_mem + range_mem
    }
}
