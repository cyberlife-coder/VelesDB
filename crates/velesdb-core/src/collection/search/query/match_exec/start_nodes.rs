//! Start-node resolution and node-matching helpers for MATCH queries.
//!
//! Extracted from `match_exec/mod.rs` to reduce NLOC.
//! Contains `find_start_nodes`, label/property matching, and pattern helpers.

use crate::collection::types::Collection;
use crate::error::{Error, Result};
use crate::storage::{PayloadStorage, VectorStorage};
use crate::velesql::GraphPattern;
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
    /// The caller (`execute_match_with_context`) already holds a
    /// `payload_storage.read()` guard. This method (and its delegates
    /// `find_start_nodes_indexed`, `find_start_nodes_full_scan`) may acquire
    /// a second concurrent read lock on the same `payload_storage`. This is
    /// safe because `parking_lot::RwLock` allows unlimited concurrent readers
    /// with no poisoning or deadlock risk. Refactoring to pass the existing
    /// guard down would require changing 4+ function signatures for minimal
    /// runtime benefit (read locks are non-blocking).
    pub(super) fn find_start_nodes(
        &self,
        pattern: &GraphPattern,
    ) -> Result<Vec<(u64, HashMap<String, u64>)>> {
        let first_node = pattern
            .nodes
            .first()
            .ok_or_else(|| Error::Config("Pattern must have at least one node".to_string()))?;

        // Fast path: use label index when labels are specified.
        if !first_node.labels.is_empty() {
            let has_large = self.label_index.read().has_large_ids();
            let indexed = self.find_start_nodes_indexed(first_node);

            if has_large {
                // RoaringBitmap cannot index IDs > u32::MAX, so the bitmap
                // result may be incomplete. Fall back to a full scan and
                // merge the results to avoid silently missing large-ID nodes.
                return Ok(self.merge_with_full_scan(indexed, first_node));
            }
            return Ok(indexed);
        }

        // Slow path: full scan (no labels in pattern).
        Ok(self.find_start_nodes_full_scan(first_node))
    }

    /// Merges indexed bitmap results with a full scan to capture nodes whose
    /// IDs exceed `u32::MAX` (not representable in `RoaringBitmap`).
    fn merge_with_full_scan(
        &self,
        indexed: Vec<(u64, HashMap<String, u64>)>,
        first_node: &crate::velesql::NodePattern,
    ) -> Vec<(u64, HashMap<String, u64>)> {
        let full = self.find_start_nodes_full_scan(first_node);
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
    fn find_start_nodes_indexed(
        &self,
        first_node: &crate::velesql::NodePattern,
    ) -> Vec<(u64, HashMap<String, u64>)> {
        let label_idx = self.label_index.read();
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

        let payload_storage = self.payload_storage.read();
        bitmap
            .iter()
            .filter(|&id| {
                Self::node_matches_properties_by_id(
                    u64::from(id),
                    &first_node.properties,
                    &payload_storage,
                )
            })
            .map(|id| Self::build_start_binding(u64::from(id), first_node))
            .collect()
    }

    /// O(N) full scan fallback for start nodes (no labels, or large-ID fallback).
    fn find_start_nodes_full_scan(
        &self,
        first_node: &crate::velesql::NodePattern,
    ) -> Vec<(u64, HashMap<String, u64>)> {
        let payload_storage = self.payload_storage.read();
        let vector_storage = self.vector_storage.read();
        let needs_payload = !first_node.properties.is_empty() || !first_node.labels.is_empty();

        let mut all_ids: std::collections::HashSet<u64> =
            vector_storage.ids().into_iter().collect();
        for id in payload_storage.ids() {
            all_ids.insert(id);
        }

        all_ids
            .into_iter()
            .filter(|id| {
                Self::node_matches_pattern(*id, first_node, needs_payload, &payload_storage)
            })
            .map(|id| Self::build_start_binding(id, first_node))
            .collect()
    }

    /// Checks if a node's payload satisfies property filters.
    fn node_matches_properties_by_id(
        id: u64,
        properties: &HashMap<String, crate::velesql::Value>,
        payload_storage: &crate::storage::LogPayloadStorage,
    ) -> bool {
        let payload_opt = payload_storage.retrieve(id).ok().flatten();
        Self::node_matches_properties(payload_opt.as_ref(), properties)
    }

    /// Returns true if a node matches the label and property filters of a pattern.
    fn node_matches_pattern(
        id: u64,
        node: &crate::velesql::NodePattern,
        needs_payload: bool,
        payload_storage: &crate::storage::LogPayloadStorage,
    ) -> bool {
        if !needs_payload {
            return true;
        }
        let payload_opt = payload_storage.retrieve(id).ok().flatten();
        if !node.labels.is_empty() && !Self::node_matches_labels(payload_opt.as_ref(), &node.labels)
        {
            return false;
        }
        node.properties.is_empty()
            || Self::node_matches_properties(payload_opt.as_ref(), &node.properties)
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

    /// Checks if a node's payload matches the required labels.
    pub(super) fn node_matches_labels(
        payload: Option<&serde_json::Value>,
        required: &[String],
    ) -> bool {
        let Some(payload) = payload else { return false };
        let Some(labels) = payload.get("_labels").and_then(|v| v.as_array()) else {
            return false;
        };
        let node_labels: Vec<&str> = labels.iter().filter_map(|v| v.as_str()).collect();
        required.iter().all(|r| node_labels.contains(&r.as_str()))
    }

    /// Checks if a node's payload matches the required properties.
    pub(super) fn node_matches_properties(
        payload: Option<&serde_json::Value>,
        properties: &HashMap<String, crate::velesql::Value>,
    ) -> bool {
        let Some(payload) = payload else { return false };
        properties.iter().all(|(key, expected)| {
            payload
                .get(key)
                .is_some_and(|actual| Self::values_match(expected, actual))
        })
    }

    /// Computes maximum traversal depth from pattern.
    pub(super) fn compute_max_depth(pattern: &GraphPattern) -> u32 {
        let mut max_depth = 0u32;

        for rel in &pattern.relationships {
            if let Some((_, end)) = rel.range {
                max_depth = max_depth.saturating_add(end.min(10));
            } else {
                max_depth = max_depth.saturating_add(1);
            }
        }

        if max_depth == 0 && !pattern.relationships.is_empty() {
            max_depth = u32::try_from(pattern.relationships.len()).unwrap_or(10);
        }

        max_depth.min(10)
    }

    /// Extracts relationship type filters from pattern.
    pub(super) fn extract_rel_types(pattern: &GraphPattern) -> Vec<String> {
        let mut types = Vec::new();
        for rel in &pattern.relationships {
            types.extend(rel.types.clone());
        }
        types
    }

    /// Compares a VelesQL Value with a JSON value.
    pub(super) fn values_match(
        velesql_value: &crate::velesql::Value,
        json_value: &serde_json::Value,
    ) -> bool {
        use crate::velesql::Value;

        match (velesql_value, json_value) {
            (Value::String(s), serde_json::Value::String(js)) => s == js,
            (Value::Integer(i), serde_json::Value::Number(n)) => {
                n.as_i64().is_some_and(|ni| *i == ni)
            }
            (Value::UnsignedInteger(u), serde_json::Value::Number(n)) => {
                n.as_u64().is_some_and(|nu| *u == nu)
            }
            (Value::Float(f), serde_json::Value::Number(n)) => {
                n.as_f64().is_some_and(|nf| (*f - nf).abs() < 0.001)
            }
            (Value::Boolean(b), serde_json::Value::Bool(jb)) => b == jb,
            (Value::Null, serde_json::Value::Null) => true,
            _ => false,
        }
    }
}
