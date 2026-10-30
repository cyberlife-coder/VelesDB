//! Range indexes and edge property indexes (EPIC-047 US-002, US-003).
//!
//! Provides B-tree based range indexes for ordered queries on node and edge properties.
//!
//! `CompositeRangeIndex` and `EdgePropertyIndex` share the same underlying
//! B-tree storage via [`BTreeOrderedIndex`], differing only in their metadata
//! (label vs relationship-type) and public API surface.

// EPIC-047 US-002/US-003 — range indexes and edge property indexes.
// Index population is wired via graph_property_index_wiring.rs;
// query-side lookups are wired but not yet called from MATCH pipeline.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::ops::Bound;

// =============================================================================
// OrderedValue: total ordering wrapper for JSON values
// =============================================================================

/// Wrapper for total ordering on JSON values.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderedValue(pub(crate) Value);

impl PartialEq for OrderedValue {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl Eq for OrderedValue {}

impl PartialOrd for OrderedValue {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for OrderedValue {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Compare by type first, then by value
        match (&self.0, &other.0) {
            (Value::Null, Value::Null) => std::cmp::Ordering::Equal,
            (Value::Null, _) => std::cmp::Ordering::Less,
            (_, Value::Null) => std::cmp::Ordering::Greater,
            (Value::Number(a), Value::Number(b)) => {
                let a_f = a.as_f64().unwrap_or(0.0);
                let b_f = b.as_f64().unwrap_or(0.0);
                a_f.total_cmp(&b_f)
            }
            (Value::String(a), Value::String(b)) => a.cmp(b),
            (Value::Bool(a), Value::Bool(b)) => a.cmp(b),
            _ => serde_json::to_string(&self.0)
                .unwrap_or_default()
                .cmp(&serde_json::to_string(&other.0).unwrap_or_default()),
        }
    }
}

// =============================================================================
// BTreeOrderedIndex: shared B-tree storage backing both index types
// =============================================================================

/// Shared B-tree storage for ordered property indexes.
///
/// Both [`CompositeRangeIndex`] (node properties) and [`EdgePropertyIndex`]
/// (edge properties) delegate to this type for all insert/remove/lookup
/// operations, eliminating code duplication.
#[derive(Debug, Serialize, Deserialize)]
struct BTreeOrderedIndex {
    entries: std::collections::BTreeMap<OrderedValue, Vec<u64>>,
}

impl BTreeOrderedIndex {
    fn new() -> Self {
        Self {
            entries: std::collections::BTreeMap::new(),
        }
    }

    fn insert(&mut self, id: u64, value: &Value) {
        self.entries
            .entry(OrderedValue(value.clone()))
            .or_default()
            .push(id);
    }

    fn remove(&mut self, id: u64, value: &Value) -> bool {
        let key = OrderedValue(value.clone());
        if let Some(ids) = self.entries.get_mut(&key) {
            if let Some(pos) = ids.iter().position(|&stored| stored == id) {
                ids.swap_remove(pos);
                if ids.is_empty() {
                    self.entries.remove(&key);
                }
                return true;
            }
        }
        false
    }

    fn lookup_exact(&self, value: &Value) -> &[u64] {
        self.entries
            .get(&OrderedValue(value.clone()))
            .map_or(&[], Vec::as_slice)
    }

    fn collect_range(&self, start: Bound<OrderedValue>, end: Bound<OrderedValue>) -> Vec<u64> {
        self.entries
            .range((start, end))
            .flat_map(|(_, ids)| ids.iter().copied())
            .collect()
    }

    fn lookup_range(&self, lower: Option<&Value>, upper: Option<&Value>) -> Vec<u64> {
        let start = match lower {
            Some(v) => Bound::Included(OrderedValue(v.clone())),
            None => Bound::Unbounded,
        };
        let end = match upper {
            Some(v) => Bound::Included(OrderedValue(v.clone())),
            None => Bound::Unbounded,
        };
        self.collect_range(start, end)
    }

    fn lookup_gt(&self, value: &Value) -> Vec<u64> {
        self.collect_range(
            Bound::Excluded(OrderedValue(value.clone())),
            Bound::Unbounded,
        )
    }

    fn lookup_lt(&self, value: &Value) -> Vec<u64> {
        self.collect_range(
            Bound::Unbounded,
            Bound::Excluded(OrderedValue(value.clone())),
        )
    }
}

// =============================================================================
// EPIC-047 US-002: Range Index (B-tree based)
// =============================================================================

/// B-tree based range index for ordered queries on node properties.
#[derive(Debug, Serialize, Deserialize)]
pub struct CompositeRangeIndex {
    /// Label this index covers
    label: String,
    /// Property name
    property: String,
    /// Shared B-tree storage
    index: BTreeOrderedIndex,
}

impl CompositeRangeIndex {
    /// Creates a new range index.
    #[must_use]
    pub fn new(label: impl Into<String>, property: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            property: property.into(),
            index: BTreeOrderedIndex::new(),
        }
    }

    /// Returns the label.
    #[must_use]
    #[allow(dead_code)] // Reason: Public API accessor — used by tests and future query planner integration
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Returns the property.
    #[must_use]
    #[allow(dead_code)] // Reason: Public API accessor — used by tests and future query planner integration
    pub fn property(&self) -> &str {
        &self.property
    }

    /// Inserts a node into the index.
    pub fn insert(&mut self, node_id: u64, value: &Value) {
        self.index.insert(node_id, value);
    }

    /// Removes a node from the index.
    pub fn remove(&mut self, node_id: u64, value: &Value) -> bool {
        self.index.remove(node_id, value)
    }

    /// Looks up nodes by exact value.
    #[must_use]
    pub fn lookup_exact(&self, value: &Value) -> &[u64] {
        self.index.lookup_exact(value)
    }

    /// Range lookup: returns nodes where value is in [lower, upper].
    pub fn lookup_range(&self, lower: Option<&Value>, upper: Option<&Value>) -> Vec<u64> {
        self.index.lookup_range(lower, upper)
    }

    /// Greater than lookup.
    pub fn lookup_gt(&self, value: &Value) -> Vec<u64> {
        self.index.lookup_gt(value)
    }

    /// Less than lookup.
    pub fn lookup_lt(&self, value: &Value) -> Vec<u64> {
        self.index.lookup_lt(value)
    }
}

// =============================================================================
// EPIC-047 US-003: Edge Property Index
// =============================================================================

/// Index for edge/relationship properties.
#[derive(Debug, Serialize, Deserialize)]
pub struct EdgePropertyIndex {
    /// Relationship type this index covers
    rel_type: String,
    /// Property name
    property: String,
    /// Shared B-tree storage
    index: BTreeOrderedIndex,
}

impl EdgePropertyIndex {
    /// Creates a new edge property index.
    #[must_use]
    pub fn new(rel_type: impl Into<String>, property: impl Into<String>) -> Self {
        Self {
            rel_type: rel_type.into(),
            property: property.into(),
            index: BTreeOrderedIndex::new(),
        }
    }

    /// Returns the relationship type.
    #[must_use]
    #[allow(dead_code)] // Reason: Public API accessor — used by tests and future query planner integration
    pub fn rel_type(&self) -> &str {
        &self.rel_type
    }

    /// Returns the property.
    #[must_use]
    #[allow(dead_code)] // Reason: Public API accessor — used by tests and future query planner integration
    pub fn property(&self) -> &str {
        &self.property
    }

    /// Inserts an edge into the index.
    pub fn insert(&mut self, edge_id: u64, value: &Value) {
        self.index.insert(edge_id, value);
    }

    /// Removes an edge from the index.
    #[allow(dead_code)] // Reason: Public API — used by tests and future edge removal wiring
    pub fn remove(&mut self, edge_id: u64, value: &Value) -> bool {
        self.index.remove(edge_id, value)
    }

    /// Looks up edges by exact value.
    #[must_use]
    #[allow(dead_code)] // Reason: Public API — used by tests and future query planner integration
    pub fn lookup_exact(&self, value: &Value) -> &[u64] {
        self.index.lookup_exact(value)
    }

    /// Range lookup for edges.
    #[allow(dead_code)] // Reason: Public API — used by tests and future query planner integration
    pub fn lookup_range(&self, lower: Option<&Value>, upper: Option<&Value>) -> Vec<u64> {
        self.index.lookup_range(lower, upper)
    }
}
