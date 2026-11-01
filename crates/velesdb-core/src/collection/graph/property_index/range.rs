//! Range indexes on node properties (EPIC-047 US-002).
//!
//! Provides B-tree based range indexes for ordered queries on node properties.
//! `CompositeRangeIndex` keeps its entries in a [`BTreeOrderedIndex`].

// Index population is wired via graph_property_index_wiring.rs;
// lookups serve the MATCH prefilter (search/query/match_exec/index_prefilter.rs).

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::ops::Bound;

// =============================================================================
// OrderedValue: total ordering wrapper for JSON values
// =============================================================================

/// Wrapper for total ordering on JSON values.
///
/// Numbers are ordered by their `f64` value with `-0.0 == 0.0`, as a MATCH
/// WHERE compares them, so an index lookup on a bound never leaves out a
/// value WHERE accepts. Equality is the ordering's: two numbers with the same
/// `f64` value are one key.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderedValue(pub(crate) Value);

impl PartialEq for OrderedValue {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == std::cmp::Ordering::Equal
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
                // A JSON number is never NaN, so `partial_cmp` is total here;
                // unlike `total_cmp` it puts `-0.0` and `0.0` level.
                a_f.partial_cmp(&b_f).unwrap_or(std::cmp::Ordering::Equal)
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
// BTreeOrderedIndex: B-tree storage backing CompositeRangeIndex
// =============================================================================

/// B-tree storage for an ordered property index: [`CompositeRangeIndex`]
/// delegates every insert, remove and lookup to it.
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
    #[cfg(test)]
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Returns the property.
    #[must_use]
    #[cfg(test)]
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
