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
/// `RoaringBitmap` only supports u32 IDs. Node IDs exceeding `u32::MAX` are
/// skipped: counted (and logged at DEBUG) when the node carries `_labels` —
/// an unlabeled node had nothing to index, so its oversized id costs
/// nothing and is not an event. Per-node WARN here used to describe the
/// NOMINAL state of a memory store (hashed u64 ids are all but guaranteed
/// past `u32::MAX` — 788 lines on one startup) and drowned the incident
/// log #1780 exists to keep readable; the caller that rebuilds the index
/// emits ONE aggregated warning instead (#1834).
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
    /// How many LABELED nodes an `index_from_payload` call skipped for an ID
    /// exceeding `u32::MAX`. Callers should fall back to a full scan when
    /// this is non-zero and the bitmap lookup returns no results; the index
    /// rebuild reports it once, aggregated, instead of one line per node
    /// (#1834). Unlabeled oversized ids are not counted: they had nothing
    /// to index, so nothing was lost.
    unindexable_labeled: u64,
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
        let labels = payload.get("_labels").and_then(|v| v.as_array());
        let Some(safe_id) = safe_bitmap_id(node_id) else {
            // Count (and detail at DEBUG) only when something was actually
            // lost — the node carried labels this index cannot hold. The
            // caller falls back to a full scan off the counter, and the
            // rebuild reports the total ONCE instead of per node (#1834).
            if labels.is_some() {
                self.unindexable_labeled += 1;
                tracing::debug!(
                    node_id,
                    "LabelIndex: labeled node_id exceeds u32::MAX, not indexed — \
                     label lookups fall back to a full scan"
                );
            }
            return 0;
        };

        let Some(labels_arr) = labels else {
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
        self.unindexable_labeled > 0
    }

    /// How many labeled nodes were skipped for an ID exceeding `u32::MAX` —
    /// what the index rebuild reports once, aggregated, instead of one WARN
    /// per node (#1834). Unlabeled oversized ids are not counted: they had
    /// nothing to index.
    #[must_use]
    pub fn unindexable_labeled(&self) -> u64 {
        self.unindexable_labeled
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
            // A missing label means an empty intersection: `?` returns None.
            let bitmap = self.labels.get(label.as_str())?;
            result &= bitmap;
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
