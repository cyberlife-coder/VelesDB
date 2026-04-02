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

    /// Drops a secondary metadata index for a payload field.
    ///
    /// Returns `true` if the index existed and was removed, `false` if no
    /// such index existed.
    #[must_use]
    pub fn drop_secondary_index(&self, field_name: &str) -> bool {
        self.secondary_indexes.write().remove(field_name).is_some()
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

    /// Builds a pre-filter bitmap from a [`Filter`] using secondary indexes.
    ///
    /// Supports `Eq`, `Gt`/`Gte`/`Lt`/`Lte` (range scan), `And` (intersection),
    /// and `Or` (union, only when all children resolve). Returns `None` when the
    /// condition cannot be resolved via indexes (e.g., `Not`, `Neq`, non-indexed
    /// fields), signalling the caller to fall back to post-filter.
    #[must_use]
    pub(crate) fn build_prefilter_bitmap(
        &self,
        filter: &crate::filter::Filter,
    ) -> Option<roaring::RoaringBitmap> {
        Self::bitmap_from_condition(&self.secondary_indexes, &filter.condition)
    }

    /// Recursively extracts bitmaps from conditions backed by secondary indexes.
    ///
    /// Supported conditions:
    /// - `Eq`: exact-match lookup
    /// - `Gt`, `Gte`, `Lt`, `Lte`: range scan via `BTreeMap::range()`
    /// - `And`: intersection of child bitmaps
    /// - `Or`: union of child bitmaps (all children must resolve)
    ///
    /// Returns `None` for `Not`, `Neq`, and unsupported conditions
    /// (these require a universe bitmap that is not yet implemented).
    fn bitmap_from_condition(
        indexes: &std::sync::Arc<
            parking_lot::RwLock<std::collections::HashMap<String, SecondaryIndex>>,
        >,
        cond: &crate::filter::Condition,
    ) -> Option<roaring::RoaringBitmap> {
        match cond {
            crate::filter::Condition::Eq { field, value } => {
                Self::bitmap_for_eq_field(indexes, field, value)
            }
            crate::filter::Condition::Gt { field, value }
            | crate::filter::Condition::Gte { field, value }
            | crate::filter::Condition::Lt { field, value }
            | crate::filter::Condition::Lte { field, value } => {
                Self::bitmap_for_range_field(indexes, field, value, cond)
            }
            crate::filter::Condition::And { conditions } => {
                Self::bitmap_from_and(indexes, conditions)
            }
            crate::filter::Condition::Or { conditions } => {
                Self::bitmap_from_or(indexes, conditions)
            }
            // TODO #487: NOT and Neq need a universe bitmap to invert.
            // Post-filter handles these correctly until then.
            _ => None,
        }
    }

    /// Looks up a single equality field in the secondary indexes.
    fn bitmap_for_eq_field(
        indexes: &std::sync::Arc<
            parking_lot::RwLock<std::collections::HashMap<String, SecondaryIndex>>,
        >,
        field: &str,
        value: &serde_json::Value,
    ) -> Option<roaring::RoaringBitmap> {
        let key = JsonValue::from_json(value)?;
        let guard = indexes.read();
        let index = guard.get(field)?;
        let bm = index.to_bitmap(&key);
        if bm.is_empty() {
            return Some(bm); // Empty bitmap = no matches (valid pre-filter)
        }
        Some(bm)
    }

    /// Builds a range bitmap for Gt/Gte/Lt/Lte using `SecondaryIndex::range_bitmap`.
    fn bitmap_for_range_field(
        indexes: &std::sync::Arc<
            parking_lot::RwLock<std::collections::HashMap<String, SecondaryIndex>>,
        >,
        field: &str,
        value: &serde_json::Value,
        cond: &crate::filter::Condition,
    ) -> Option<roaring::RoaringBitmap> {
        use std::ops::Bound;

        let key = JsonValue::from_json(value)?;
        let guard = indexes.read();
        let index = guard.get(field)?;
        let (from, to) = match cond {
            crate::filter::Condition::Gt { .. } => (Bound::Excluded(&key), Bound::Unbounded),
            crate::filter::Condition::Gte { .. } => (Bound::Included(&key), Bound::Unbounded),
            crate::filter::Condition::Lt { .. } => (Bound::Unbounded, Bound::Excluded(&key)),
            crate::filter::Condition::Lte { .. } => (Bound::Unbounded, Bound::Included(&key)),
            _ => return None,
        };
        Some(index.range_bitmap(from, to))
    }

    /// Intersects bitmaps from AND-ed conditions.
    fn bitmap_from_and(
        indexes: &std::sync::Arc<
            parking_lot::RwLock<std::collections::HashMap<String, SecondaryIndex>>,
        >,
        conditions: &[crate::filter::Condition],
    ) -> Option<roaring::RoaringBitmap> {
        let mut result: Option<roaring::RoaringBitmap> = None;
        for cond in conditions {
            if let Some(bm) = Self::bitmap_from_condition(indexes, cond) {
                result = Some(match result {
                    Some(existing) => existing & &bm,
                    None => bm,
                });
            }
        }
        result
    }

    /// Unions bitmaps from OR-ed conditions.
    ///
    /// If ANY child returns `None` (cannot be pre-filtered), the entire OR
    /// must return `None` because the union would be incomplete -- the
    /// post-filter must evaluate the full OR instead.
    fn bitmap_from_or(
        indexes: &std::sync::Arc<
            parking_lot::RwLock<std::collections::HashMap<String, SecondaryIndex>>,
        >,
        conditions: &[crate::filter::Condition],
    ) -> Option<roaring::RoaringBitmap> {
        let mut result = roaring::RoaringBitmap::new();
        for cond in conditions {
            let bm = Self::bitmap_from_condition(indexes, cond)?;
            result |= bm;
        }
        Some(result)
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

        // LOCK ORDER: property_index(7) read — then range_index(7) read.
        // Same level, reads-only; canonical order prevents deadlock.
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
        // LOCK ORDER: property_index(7) read — then range_index(7) read.
        // Same level, reads-only; canonical order prevents deadlock.
        let prop_mem = self.property_index.read().memory_usage();
        let range_mem = self.range_index.read().memory_usage();
        prop_mem + range_mem
    }
}
