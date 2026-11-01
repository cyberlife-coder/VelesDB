//! Wiring logic for graph property indexes (EPIC-047 Wave 7).
//!
//! Populates the node range indexes (`CompositeRangeIndex`) when node payloads
//! change.
//!
//! # The node range indexes are derived from the payloads
//!
//! A MATCH range predicate takes its candidate nodes from the
//! `"label.property"` range index whenever that index exists, so an index
//! missing a node hides the node from the result. The range indexes are
//! therefore maintained exactly like the label index, from the same payloads
//! and at the same points: [`Collection::apply_range_index_changes`] runs
//! wherever a write or delete path updates the label index, and the open
//! rebuilds both from the stored payloads in one scan
//! (`lifecycle.rs`, `rebuild_payload_indexes`), through the same
//! [`index_payload_ranges`].

use crate::collection::graph::property_index::CompositeRangeIndex;
use crate::collection::types::Collection;
use serde_json::Value;
use std::collections::HashMap;

/// The node range indexes, keyed by `"label.property"`.
pub(crate) type GraphRangeIndexes = HashMap<String, CompositeRangeIndex>;

/// One node's payload change, as the derived indexes see it: the node id,
/// the payload it carried before the write (`None` for a new node), and the
/// one it carries after (`None` for a delete).
pub(crate) type NodePayloadChange<'a> = (u64, Option<&'a Value>, Option<&'a Value>);

/// Key format for graph range indexes: `"label.property"`.
fn range_index_key(label: &str, property: &str) -> String {
    format!("{label}.{property}")
}

/// Extracts the `_labels` array from a node payload.
pub(crate) fn extract_labels(payload: &Value) -> Vec<String> {
    payload
        .get("_labels")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// The labels of a node payload, borrowed: the `_labels` array's strings.
fn payload_labels(payload: &Value) -> impl Iterator<Item = &str> {
    payload
        .get("_labels")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
}

/// The `(property, value)` pairs of a payload a range index holds: top-level
/// scalars (null, boolean, number, string) under a key without a `.`.
///
/// Both exclusions keep an entry only where an index-eligible WHERE can
/// match it, so leaving the rest out never hides a node. An array or an
/// object never satisfies an extracted comparison (`evaluate_comparison`
/// and the filter engine's `BETWEEN` both reject them), and indexing them
/// copied every embedded vector and nested document of every labelled node.
/// A dotted top-level key is unreachable: WHERE reads `n.a.b` as the nested
/// path `a`, then `b`, and the index key `label.a.b` it is looked up under
/// would otherwise hold the dotted key's nodes and hide the nested ones.
fn range_indexable_properties(payload: &Value) -> impl Iterator<Item = (&str, &Value)> {
    payload
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(key, value)| {
            *key != "_labels" && !key.contains('.') && !value.is_array() && !value.is_object()
        })
        .map(|(key, value)| (key.as_str(), value))
}

/// Writes the `"label.property"` key into `buf`, reusing its allocation.
fn write_range_key(buf: &mut String, label: &str, property: &str) {
    buf.clear();
    buf.push_str(label);
    buf.push('.');
    buf.push_str(property);
}

/// Adds every labelled scalar property of `payload` to `indexes` for
/// `node_id`, creating the `"label.property"` index on first sight.
///
/// The one insertion step shared by the write paths
/// ([`Collection::apply_range_index_changes`]) and the open-time rebuild, so a
/// reopened collection holds exactly the entries the writes had made.
pub(crate) fn index_payload_ranges(indexes: &mut GraphRangeIndexes, node_id: u64, payload: &Value) {
    let mut key = String::new();
    for label in payload_labels(payload) {
        for (property, value) in range_indexable_properties(payload) {
            write_range_key(&mut key, label, property);
            if let Some(index) = indexes.get_mut(key.as_str()) {
                index.insert(node_id, value);
            } else {
                let mut index = CompositeRangeIndex::new(label, property);
                index.insert(node_id, value);
                indexes.insert(key.clone(), index);
            }
        }
    }
}

/// Removes the entries [`index_payload_ranges`] made for `node_id` from
/// `payload`, the payload the node carried until now.
fn unindex_payload_ranges(indexes: &mut GraphRangeIndexes, node_id: u64, payload: &Value) {
    let mut key = String::new();
    for label in payload_labels(payload) {
        for (property, value) in range_indexable_properties(payload) {
            write_range_key(&mut key, label, property);
            if let Some(index) = indexes.get_mut(key.as_str()) {
                let _removed = index.remove(node_id, value);
            }
        }
    }
}

/// Whether `payload` exists and carries `_labels`: a payload without labels
/// has no range-index entry to add or remove.
fn carries_labels(payload: Option<&Value>) -> bool {
    payload.is_some_and(|value| value.get("_labels").is_some())
}

impl Collection {
    /// Applies node payload changes to the graph range indexes, taking their
    /// write lock once for the whole batch.
    ///
    /// Each change removes what the old payload indexed before indexing the
    /// new one — the sequence every path runs on the label index — so a
    /// property carried on both sides of an update is not added and then
    /// taken straight back out. Callers pass the effective old payload of a
    /// duplicate id within one batch (the previous occurrence, not the
    /// pre-batch value), exactly as they do for the label index.
    ///
    /// LOCK ORDER: graph_range_indexes(7). `label_index` is also rank 7, with
    /// no order between the two, so callers release it before calling this.
    pub(crate) fn apply_range_index_changes<'a>(
        &self,
        changes: impl IntoIterator<Item = NodePayloadChange<'a>>,
    ) {
        let mut changes = changes
            .into_iter()
            .filter(|&(_, old, new)| carries_labels(old) || carries_labels(new))
            .peekable();
        // Unlabelled batches (every plain vector upsert) never take the lock.
        if changes.peek().is_none() {
            return;
        }
        let mut indexes = self.graph.graph_range_indexes.write();
        for (node_id, old, new) in changes {
            if let Some(old) = old {
                unindex_payload_ranges(&mut indexes, node_id, old);
            }
            if let Some(new) = new {
                index_payload_ranges(&mut indexes, node_id, new);
            }
        }
    }

    /// Looks up the range index for `label.property` and, if present, runs
    /// `f` against it. Shared by the `graph_range_lookup_*` variants below,
    /// which differ only in which `CompositeRangeIndex` method `f` calls.
    fn with_range_index<T>(
        &self,
        label: &str,
        property: &str,
        f: impl FnOnce(&CompositeRangeIndex) -> T,
    ) -> Option<T> {
        let key = range_index_key(label, property);
        let indexes = self.graph.graph_range_indexes.read();
        indexes.get(&key).map(f)
    }

    /// Looks up node IDs matching a greater-than predicate.
    #[must_use]
    pub(crate) fn graph_range_lookup_gt(
        &self,
        label: &str,
        property: &str,
        value: &Value,
    ) -> Option<Vec<u64>> {
        self.with_range_index(label, property, |idx| idx.lookup_gt(value))
    }

    /// Looks up node IDs matching a less-than predicate.
    #[must_use]
    pub(crate) fn graph_range_lookup_lt(
        &self,
        label: &str,
        property: &str,
        value: &Value,
    ) -> Option<Vec<u64>> {
        self.with_range_index(label, property, |idx| idx.lookup_lt(value))
    }

    /// Looks up node IDs matching a range predicate.
    #[must_use]
    pub(crate) fn graph_range_lookup(
        &self,
        label: &str,
        property: &str,
        lower: Option<&Value>,
        upper: Option<&Value>,
    ) -> Option<Vec<u64>> {
        self.with_range_index(label, property, |idx| idx.lookup_range(lower, upper))
    }

    /// Looks up node IDs matching an exact value.
    #[must_use]
    pub(crate) fn graph_range_lookup_exact(
        &self,
        label: &str,
        property: &str,
        value: &Value,
    ) -> Option<Vec<u64>> {
        self.with_range_index(label, property, |idx| idx.lookup_exact(value).to_vec())
    }
}
