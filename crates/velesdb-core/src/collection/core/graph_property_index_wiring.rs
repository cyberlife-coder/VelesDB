//! Wiring logic for graph property indexes (EPIC-047 Wave 7).
//!
//! Populates `CompositeRangeIndex`, `EdgePropertyIndex`, `CompositeIndexManager`,
//! and `QueryPatternTracker` when nodes and edges are mutated.

use crate::collection::graph::property_index::{
    CompositeRangeIndex, EdgePropertyIndex, IndexSuggestion,
};
use crate::collection::types::Collection;
use serde_json::Value;
use std::collections::HashMap;

/// Key format for graph range indexes: `"label.property"`.
fn range_index_key(label: &str, property: &str) -> String {
    format!("{label}.{property}")
}

/// Key format for edge range indexes: `"rel_type.property"`.
fn edge_index_key(rel_type: &str, property: &str) -> String {
    format!("{rel_type}.{property}")
}

/// Extracts the `_labels` array from a node payload.
fn extract_labels(payload: &Value) -> Vec<String> {
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

/// Returns a vec of `(property_name, value)` from a payload,
/// excluding the internal `_labels` key.
fn indexable_properties(payload: &Value) -> Vec<(&str, &Value)> {
    payload
        .as_object()
        .map(|obj| {
            obj.iter()
                .filter(|(k, _)| *k != "_labels")
                .map(|(k, v)| (k.as_str(), v))
                .collect()
        })
        .unwrap_or_default()
}

impl Collection {
    /// Populates graph property indexes after a node payload is stored.
    pub(crate) fn index_node_properties(&self, node_id: u64, payload: &Value) {
        let labels = extract_labels(payload);
        let properties = indexable_properties(payload);

        if labels.is_empty() || properties.is_empty() {
            return;
        }

        // Populate range indexes
        {
            let mut range_indexes = self.graph_range_indexes.write();
            for label in &labels {
                for &(prop_name, prop_value) in &properties {
                    let key = range_index_key(label, prop_name);
                    range_indexes
                        .entry(key)
                        .or_insert_with(|| {
                            CompositeRangeIndex::new(label.clone(), prop_name.to_string())
                        })
                        .insert(node_id, prop_value);
                }
            }
        }

        // Populate composite indexes
        {
            let mut composite_mgr = self.composite_index_manager.write();
            for label in &labels {
                let props_map: HashMap<String, Value> = properties
                    .iter()
                    .map(|&(k, v)| (k.to_string(), v.clone()))
                    .collect();
                composite_mgr.on_add_node(label, node_id, &props_map);
            }
        }
    }

    /// Removes a node from graph property indexes before an update.
    pub(crate) fn deindex_node_properties(&self, node_id: u64, old_payload: &Value) {
        let labels = extract_labels(old_payload);
        let properties = indexable_properties(old_payload);

        if labels.is_empty() || properties.is_empty() {
            return;
        }

        {
            let mut range_indexes = self.graph_range_indexes.write();
            for label in &labels {
                for &(prop_name, prop_value) in &properties {
                    let key = range_index_key(label, prop_name);
                    if let Some(idx) = range_indexes.get_mut(&key) {
                        let _removed = idx.remove(node_id, prop_value);
                    }
                }
            }
        }

        {
            let mut composite_mgr = self.composite_index_manager.write();
            for label in &labels {
                let props_map: HashMap<String, Value> = properties
                    .iter()
                    .map(|&(k, v)| (k.to_string(), v.clone()))
                    .collect();
                composite_mgr.on_remove_node(label, node_id, &props_map);
            }
        }
    }

    /// Populates edge property indexes after an edge is added.
    pub(crate) fn index_edge_properties(
        &self,
        edge_id: u64,
        rel_type: &str,
        properties: &HashMap<String, Value>,
    ) {
        if properties.is_empty() {
            return;
        }

        let mut edge_indexes = self.edge_range_indexes.write();
        for (prop_name, prop_value) in properties {
            let key = edge_index_key(rel_type, prop_name);
            edge_indexes
                .entry(key)
                .or_insert_with(|| EdgePropertyIndex::new(rel_type.to_string(), prop_name.clone()))
                .insert(edge_id, prop_value);
        }
    }

    /// Records a query pattern for the index advisor.
    pub(crate) fn record_query_pattern(
        &self,
        labels: Vec<String>,
        properties: Vec<String>,
        predicates: Vec<crate::collection::graph::property_index::PredicateType>,
        execution_time_ms: u64,
    ) {
        use crate::collection::graph::property_index::QueryPattern;

        if labels.is_empty() || properties.is_empty() {
            return;
        }

        let pattern = QueryPattern {
            labels,
            properties,
            predicates,
        };

        self.query_pattern_tracker
            .write()
            .record(pattern, execution_time_ms);
    }

    /// Returns index suggestions based on tracked query patterns.
    #[must_use]
    #[allow(dead_code)] // Reason: Public API for query evaluator — called when MATCH pipeline integrates auto-suggestion
    pub(crate) fn index_suggestions(&self) -> Vec<IndexSuggestion> {
        let tracker = self.query_pattern_tracker.read();
        self.index_advisor.read().suggest(&tracker)
    }

    /// Looks up node IDs matching a greater-than predicate.
    #[must_use]
    pub(crate) fn graph_range_lookup_gt(
        &self,
        label: &str,
        property: &str,
        value: &Value,
    ) -> Option<Vec<u64>> {
        let key = range_index_key(label, property);
        let indexes = self.graph_range_indexes.read();
        indexes
            .get(&key)
            .map(|idx: &CompositeRangeIndex| idx.lookup_gt(value))
    }

    /// Looks up node IDs matching a less-than predicate.
    #[must_use]
    pub(crate) fn graph_range_lookup_lt(
        &self,
        label: &str,
        property: &str,
        value: &Value,
    ) -> Option<Vec<u64>> {
        let key = range_index_key(label, property);
        let indexes = self.graph_range_indexes.read();
        indexes
            .get(&key)
            .map(|idx: &CompositeRangeIndex| idx.lookup_lt(value))
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
        let key = range_index_key(label, property);
        let indexes = self.graph_range_indexes.read();
        indexes
            .get(&key)
            .map(|idx: &CompositeRangeIndex| idx.lookup_range(lower, upper))
    }

    /// Looks up node IDs matching an exact value.
    #[must_use]
    pub(crate) fn graph_range_lookup_exact(
        &self,
        label: &str,
        property: &str,
        value: &Value,
    ) -> Option<Vec<u64>> {
        let key = range_index_key(label, property);
        let indexes = self.graph_range_indexes.read();
        indexes
            .get(&key)
            .map(|idx: &CompositeRangeIndex| idx.lookup_exact(value).to_vec())
    }

    /// Looks up node IDs via composite index for multi-property equality.
    #[must_use]
    pub(crate) fn composite_index_lookup(
        &self,
        label: &str,
        properties: &[&str],
        values: &[Value],
    ) -> Option<Vec<u64>> {
        let mgr = self.composite_index_manager.read();
        let covering = mgr.find_covering_indexes(label, properties);
        let index_name = covering.first()?;
        let idx = mgr.get(index_name)?;
        Some(idx.lookup(values).to_vec())
    }
}
