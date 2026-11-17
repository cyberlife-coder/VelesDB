//! Start-node resolution and node-matching helpers for MATCH queries.
//!
//! Extracted from `match_exec/mod.rs` to reduce NLOC.
//! Contains `find_start_nodes`, label/property matching, and pattern helpers.

use super::MatchStorageGuards;
use crate::collection::types::Collection;
use crate::error::{Error, Result};
use crate::storage::{PayloadStorage, VectorStorage};
use crate::velesql::{GraphPattern, NodePattern};
use std::collections::HashMap;

impl Collection {
    /// Finds start nodes matching the first node pattern.
    ///
    /// When the pattern specifies labels (e.g., `(n:Person)`), uses the
    /// `LabelIndex` bitmap intersection for O(k) lookup instead of scanning
    /// all N nodes. Falls back to full scan when no labels are specified
    /// or when the label index contains large IDs that could not be indexed
    /// in the `RoaringBitmap` (u32 limitation).
    ///
    /// # Lock note
    ///
    /// `guards` carries the `vector_storage` (rank 2) and `payload_storage`
    /// (rank 3) read guards acquired ONCE by the top frame in decree order
    /// and passed down. This method and its delegates
    /// (`find_start_nodes_indexed`, `find_start_nodes_full_scan`) must use
    /// them and never re-acquire either lock: parking_lot's task-fair
    /// `RwLock` blocks a second read acquisition on the same thread as soon
    /// as one writer is queued, so a nested `payload_storage.read()` under
    /// the already-held guard deadlocks against any concurrent writer (the
    /// same rule `graph_api.rs::validate_edge_endpoints_exist` documents for
    /// the edge-write path).
    /// The start nodes of `pattern` per `starts`: every node its first node
    /// matches ([`Self::find_start_nodes`]), or the given seeds, in order,
    /// each kept when it matches that first node exactly as a scanned node
    /// would (labels, inline properties, annotation).
    pub(super) fn resolve_start_nodes(
        &self,
        pattern: &GraphPattern,
        guards: &MatchStorageGuards<'_>,
        starts: super::StartNodes<'_>,
    ) -> Result<Vec<(u64, HashMap<String, u64>)>> {
        let super::StartNodes::Seeded(seeds) = starts else {
            return self.find_start_nodes(pattern, guards);
        };
        let first_node = pattern
            .nodes
            .first()
            .ok_or_else(|| Error::Query("Pattern must have at least one node".to_string()))?;
        Ok(seeds
            .iter()
            .filter(|&&id| Self::node_matches_bound_pattern(id, first_node, &guards.payload_memo))
            .map(|&id| Self::build_start_binding(id, first_node))
            .collect())
    }

    fn find_start_nodes(
        &self,
        pattern: &GraphPattern,
        guards: &MatchStorageGuards<'_>,
    ) -> Result<Vec<(u64, HashMap<String, u64>)>> {
        let first_node = pattern
            .nodes
            .first()
            .ok_or_else(|| Error::Query("Pattern must have at least one node".to_string()))?;

        // Fast path: use label index when labels are specified.
        if !first_node.labels.is_empty() {
            let has_large = self.graph.label_index.read().has_large_ids();
            let indexed = self.find_start_nodes_indexed(first_node, &guards.payload_memo);

            if has_large {
                // RoaringBitmap cannot index IDs > u32::MAX, so the bitmap
                // result may be incomplete. Fall back to a full scan and
                // merge the results to avoid silently missing large-ID nodes.
                return Ok(Self::merge_with_full_scan(indexed, first_node, guards));
            }
            return Ok(indexed);
        }

        // Slow path: full scan (no labels in pattern).
        Ok(Self::find_start_nodes_full_scan(first_node, guards))
    }

    /// Merges indexed bitmap results with a full scan to capture nodes whose
    /// IDs exceed `u32::MAX` (not representable in `RoaringBitmap`).
    fn merge_with_full_scan(
        indexed: Vec<(u64, HashMap<String, u64>)>,
        first_node: &crate::velesql::NodePattern,
        guards: &MatchStorageGuards<'_>,
    ) -> Vec<(u64, HashMap<String, u64>)> {
        let full = Self::find_start_nodes_full_scan(first_node, guards);
        if indexed.is_empty() {
            return full;
        }
        let existing: std::collections::HashSet<u64> = indexed.iter().map(|(id, _)| *id).collect();
        let mut merged = indexed;
        for entry in full {
            if !existing.contains(&entry.0) {
                merged.push(entry);
            }
        }
        merged
    }

    /// O(k) label-indexed lookup for start nodes.
    ///
    /// Reads through the caller's hoisted guards (`memo`) — never re-acquires
    /// the payload lock (see the lock note on [`Self::find_start_nodes`]).
    /// The bitmap decides the labels; the inline properties are checked on
    /// the node's annotated union.
    fn find_start_nodes_indexed(
        &self,
        first_node: &crate::velesql::NodePattern,
        memo: &super::PayloadMemo<'_>,
    ) -> Vec<(u64, HashMap<String, u64>)> {
        let label_idx = self.graph.label_index.read();
        let bitmap = label_idx.lookup_intersection(&first_node.labels);
        drop(label_idx);

        let Some(bitmap) = bitmap else {
            return Vec::new();
        };

        if first_node.properties.is_empty() {
            return bitmap
                .iter()
                .map(|id| Self::build_start_binding(u64::from(id), first_node))
                .collect();
        }

        bitmap
            .iter()
            .filter(|&id| {
                let id = u64::from(id);
                let (_, properties) = memo.read_unmemoized(first_node.collection.as_deref(), id);
                Self::properties_hold(id, properties.as_deref(), &first_node.properties)
            })
            .map(|id| Self::build_start_binding(u64::from(id), first_node))
            .collect()
    }

    /// O(N) full scan fallback for start nodes (no labels, or large-ID fallback).
    ///
    /// Reads only through the hoisted guards — never re-acquires
    /// `vector_storage` or `payload_storage` (see the lock note on
    /// [`Self::find_start_nodes`]) — and without memoizing: the scan reads
    /// every node once.
    fn find_start_nodes_full_scan(
        first_node: &crate::velesql::NodePattern,
        guards: &MatchStorageGuards<'_>,
    ) -> Vec<(u64, HashMap<String, u64>)> {
        let mut all_ids: std::collections::HashSet<u64> =
            guards.vector_guard.ids().into_iter().collect();
        for id in guards.payload_guard.ids() {
            all_ids.insert(id);
        }

        all_ids
            .into_iter()
            .filter(|id| {
                if Self::pattern_reads_nothing(first_node) {
                    return true;
                }
                let (own, properties) = guards
                    .payload_memo
                    .read_unmemoized(first_node.collection.as_deref(), *id);
                Self::payloads_match_pattern(*id, own.as_deref(), properties.as_deref(), first_node)
            })
            .map(|id| Self::build_start_binding(id, first_node))
            .collect()
    }

    /// Returns true if a bound node satisfies its pattern: its labels, read
    /// on its own payload, and its inline properties, read on the node as
    /// its `@collection` annotation unites it (see [`super::sources`]).
    pub(super) fn node_matches_bound_pattern(
        id: u64,
        node: &NodePattern,
        memo: &super::PayloadMemo<'_>,
    ) -> bool {
        if Self::pattern_reads_nothing(node) {
            return true;
        }
        // Bound-node checks repeat per hop candidacy: read through the memo.
        let own = memo.get(id);
        let properties = if node.properties.is_empty() {
            None
        } else {
            memo.get_annotated(node.collection.as_deref(), id)
        };
        Self::payloads_match_pattern(id, own.as_deref(), properties.as_deref(), node)
    }

    /// Whether `node` has neither labels nor inline properties to check.
    fn pattern_reads_nothing(node: &NodePattern) -> bool {
        node.labels.is_empty() && node.properties.is_empty()
    }

    /// Label check against the node's own payload, property check against
    /// its (possibly annotated) property payload — both as every MATCH
    /// engine decides them ([`crate::velesql::has_every_label`],
    /// [`crate::velesql::inline_properties_hold`]).
    fn payloads_match_pattern(
        id: u64,
        own: Option<&serde_json::Value>,
        properties: Option<&serde_json::Value>,
        node: &NodePattern,
    ) -> bool {
        crate::velesql::has_every_label(&node.labels, &labels_of(own))
            && Self::properties_hold(id, properties, &node.properties)
    }

    /// Whether node `id`, whose properties `payload` holds, satisfies the
    /// inline properties `properties`.
    fn properties_hold(
        id: u64,
        payload: Option<&serde_json::Value>,
        properties: &HashMap<String, crate::velesql::Value>,
    ) -> bool {
        crate::velesql::inline_properties_hold(properties, |key| {
            crate::velesql::element_property(id, key, |k| payload?.get(k))
        })
    }

    /// Builds a `(node_id, bindings)` pair for a start node.
    fn build_start_binding(
        id: u64,
        node: &crate::velesql::NodePattern,
    ) -> (u64, HashMap<String, u64>) {
        let mut bindings: HashMap<String, u64> = HashMap::new();
        if let Some(ref alias) = node.alias {
            bindings.insert(alias.clone(), id);
        }
        (id, bindings)
    }
}

/// The `_labels` a node's own payload carries.
fn labels_of(payload: Option<&serde_json::Value>) -> Vec<&str> {
    payload
        .and_then(|p| p.get("_labels"))
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .collect()
}
