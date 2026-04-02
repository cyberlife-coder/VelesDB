//! Label index for fast graph node lookups by label.
//!
//! Provides O(1) label-to-node-id lookups using `RoaringBitmap`, enabling
//! `find_start_nodes()` to skip the O(N) full scan when a MATCH pattern
//! specifies node labels.

use super::helpers::safe_bitmap_id;
use roaring::RoaringBitmap;
use std::collections::HashMap;

/// Index mapping label names to the set of node IDs carrying that label.
///
/// Each label maps to a `RoaringBitmap` of node IDs (u32). When a MATCH
/// query specifies `(n:Person)`, the index returns the bitmap of all
/// `Person`-labeled nodes in O(1), avoiding a full payload scan.
///
/// # Limitations
///
/// `RoaringBitmap` only supports u32 IDs. Node IDs exceeding `u32::MAX`
/// are silently skipped (logged at warn level on insert).
///
/// # Example
///
/// ```rust,ignore
/// let mut index = LabelIndex::new();
/// index.insert("Person", 1);
/// index.insert("Person", 2);
/// index.insert("Company", 3);
///
/// let persons = index.lookup("Person");
/// assert!(persons.map_or(false, |b| b.contains(1)));
/// assert!(persons.map_or(false, |b| b.contains(2)));
/// ```
#[derive(Debug, Default)]
pub struct LabelIndex {
    /// label_name -> set of node IDs with that label.
    labels: HashMap<String, RoaringBitmap>,
    /// Set to `true` when any `index_from_payload` call encounters a node ID
    /// exceeding `u32::MAX`. Callers should fall back to a full scan when this
    /// flag is set and the bitmap lookup returns no results.
    has_large_ids: bool,
}

impl LabelIndex {
    /// Creates a new empty label index.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Indexes a node under one or more labels.
    ///
    /// Extracts `_labels` from the JSON payload (expected to be an array of
    /// strings) and inserts the node ID into the bitmap for each label.
    ///
    /// Returns the number of labels successfully indexed (0 if payload has
    /// no `_labels` array or node ID exceeds `u32::MAX`).
    pub fn index_from_payload(&mut self, node_id: u64, payload: &serde_json::Value) -> usize {
        let Some(safe_id) = safe_bitmap_id(node_id) else {
            tracing::warn!(
                node_id,
                "LabelIndex: node_id exceeds u32::MAX, cannot index"
            );
            // Track that we have unindexable IDs so callers can fall back
            // to a full scan instead of returning silently incomplete results.
            if payload.get("_labels").and_then(|v| v.as_array()).is_some() {
                self.has_large_ids = true;
            }
            return 0;
        };

        let Some(labels_arr) = payload.get("_labels").and_then(|v| v.as_array()) else {
            return 0;
        };

        let mut count = 0usize;
        for label_val in labels_arr {
            if let Some(label_str) = label_val.as_str() {
                self.labels
                    .entry(label_str.to_string())
                    .or_default()
                    .insert(safe_id);
                count += 1;
            }
        }
        count
    }

    /// Inserts a single `(label, node_id)` pair into the index.
    ///
    /// Returns `true` if the node was added (new entry). Returns `false` if
    /// the node ID exceeds `u32::MAX` or was already present.
    pub fn insert(&mut self, label: &str, node_id: u64) -> bool {
        let Some(safe_id) = safe_bitmap_id(node_id) else {
            return false;
        };
        self.labels
            .entry(label.to_string())
            .or_default()
            .insert(safe_id)
    }

    /// Removes a node from all label bitmaps.
    ///
    /// Call this before removing a node to keep the index consistent.
    pub fn remove_from_payload(&mut self, node_id: u64, payload: &serde_json::Value) {
        let Some(safe_id) = safe_bitmap_id(node_id) else {
            return;
        };

        let Some(labels_arr) = payload.get("_labels").and_then(|v| v.as_array()) else {
            return;
        };

        for label_val in labels_arr {
            if let Some(label_str) = label_val.as_str() {
                if let Some(bitmap) = self.labels.get_mut(label_str) {
                    bitmap.remove(safe_id);
                    if bitmap.is_empty() {
                        self.labels.remove(label_str);
                    }
                }
            }
        }
    }

    /// Returns `true` if any node with `_labels` had an ID exceeding `u32::MAX`
    /// and could not be indexed. Callers should fall back to a full scan when
    /// this is `true` and the bitmap lookup returns empty results.
    #[must_use]
    pub fn has_large_ids(&self) -> bool {
        self.has_large_ids
    }

    /// Returns the bitmap of node IDs carrying the given label.
    ///
    /// Returns `None` if no nodes have been indexed with this label.
    #[must_use]
    pub fn lookup(&self, label: &str) -> Option<&RoaringBitmap> {
        self.labels.get(label)
    }

    /// Returns the intersection of bitmaps for all required labels.
    ///
    /// When a MATCH pattern requires multiple labels (e.g., `(n:Person:Employee)`),
    /// only nodes carrying ALL labels should match. Returns `None` if any
    /// required label has no indexed nodes (empty intersection).
    #[must_use]
    pub fn lookup_intersection(&self, labels: &[String]) -> Option<RoaringBitmap> {
        let mut iter = labels.iter();
        let first = iter.next()?;
        let mut result = self.labels.get(first.as_str())?.clone();

        for label in iter {
            match self.labels.get(label.as_str()) {
                Some(bitmap) => result &= bitmap,
                None => return None, // Label has no nodes → empty intersection
            }
            if result.is_empty() {
                return None;
            }
        }

        if result.is_empty() {
            None
        } else {
            Some(result)
        }
    }

    /// Returns the number of distinct labels in the index.
    #[must_use]
    pub fn label_count(&self) -> usize {
        self.labels.len()
    }

    /// Returns `true` if the index contains no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.labels.is_empty()
    }

    /// Clears all entries from the index.
    pub fn clear(&mut self) {
        self.labels.clear();
    }

    /// Returns an estimated memory usage in bytes.
    #[must_use]
    pub fn memory_usage(&self) -> usize {
        let mut total = std::mem::size_of::<Self>();
        for (label, bitmap) in &self.labels {
            total += label.len() + std::mem::size_of::<String>();
            total += bitmap.serialized_size();
        }
        total
    }
}
