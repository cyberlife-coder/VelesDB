//! Range indexes and edge property indexes (EPIC-047 US-002, US-003).
//!
//! Provides B-tree based range indexes for ordered queries on node and edge properties.

use roaring::RoaringBitmap;
use serde::{Deserialize, Serialize};
use serde_json::Value;

// =============================================================================
// EPIC-047 US-002: Range Index (B-tree based)
// =============================================================================

/// Wrapper for total ordering on JSON values.
// Reason: OrderedValue part of EPIC-047 range index feature
#[allow(dead_code)]
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

/// B-tree based range index for ordered queries.
#[allow(dead_code)]
#[derive(Debug, Serialize, Deserialize)]
pub struct CompositeRangeIndex {
    /// Label this index covers
    label: String,
    /// Property name
    property: String,
    /// (value) -> Vec<NodeId>
    index: std::collections::BTreeMap<OrderedValue, Vec<u64>>,
}

#[allow(dead_code)]
impl CompositeRangeIndex {
    /// Creates a new range index.
    #[must_use]
    pub fn new(label: impl Into<String>, property: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            property: property.into(),
            index: std::collections::BTreeMap::new(),
        }
    }

    /// Returns the label.
    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Returns the property.
    #[must_use]
    pub fn property(&self) -> &str {
        &self.property
    }

    /// Inserts a node into the index.
    pub fn insert(&mut self, node_id: u64, value: &Value) {
        self.index
            .entry(OrderedValue(value.clone()))
            .or_default()
            .push(node_id);
    }

    /// Removes a node from the index.
    pub fn remove(&mut self, node_id: u64, value: &Value) -> bool {
        let key = OrderedValue(value.clone());
        if let Some(nodes) = self.index.get_mut(&key) {
            if let Some(pos) = nodes.iter().position(|&id| id == node_id) {
                nodes.swap_remove(pos);
                if nodes.is_empty() {
                    self.index.remove(&key);
                }
                return true;
            }
        }
        false
    }

    /// Looks up nodes by exact value.
    #[must_use]
    pub fn lookup_exact(&self, value: &Value) -> &[u64] {
        self.index
            .get(&OrderedValue(value.clone()))
            .map_or(&[], Vec::as_slice)
    }

    /// Range lookup: returns nodes where value is in [lower, upper].
    pub fn lookup_range(&self, lower: Option<&Value>, upper: Option<&Value>) -> Vec<u64> {
        use std::ops::Bound;

        let start = match lower {
            Some(v) => Bound::Included(OrderedValue(v.clone())),
            None => Bound::Unbounded,
        };

        let end = match upper {
            Some(v) => Bound::Included(OrderedValue(v.clone())),
            None => Bound::Unbounded,
        };

        self.index
            .range((start, end))
            .flat_map(|(_, ids)| ids.iter().copied())
            .collect()
    }

    /// Greater than lookup.
    pub fn lookup_gt(&self, value: &Value) -> Vec<u64> {
        use std::ops::Bound;
        self.index
            .range((
                Bound::Excluded(OrderedValue(value.clone())),
                Bound::Unbounded,
            ))
            .flat_map(|(_, ids)| ids.iter().copied())
            .collect()
    }

    /// Less than lookup.
    pub fn lookup_lt(&self, value: &Value) -> Vec<u64> {
        use std::ops::Bound;
        self.index
            .range((
                Bound::Unbounded,
                Bound::Excluded(OrderedValue(value.clone())),
            ))
            .flat_map(|(_, ids)| ids.iter().copied())
            .collect()
    }
}

// =============================================================================
// EPIC-047 US-003: Edge Property Index
// =============================================================================

/// Index for edge/relationship properties.
#[allow(dead_code)]
#[derive(Debug, Serialize, Deserialize)]
pub struct EdgePropertyIndex {
    /// Relationship type this index covers
    rel_type: String,
    /// Property name
    property: String,
    /// (value) -> Vec<EdgeId>
    index: std::collections::BTreeMap<OrderedValue, Vec<u64>>,
}

#[allow(dead_code)]
impl EdgePropertyIndex {
    /// Creates a new edge property index.
    #[must_use]
    pub fn new(rel_type: impl Into<String>, property: impl Into<String>) -> Self {
        Self {
            rel_type: rel_type.into(),
            property: property.into(),
            index: std::collections::BTreeMap::new(),
        }
    }

    /// Returns the relationship type.
    #[must_use]
    pub fn rel_type(&self) -> &str {
        &self.rel_type
    }

    /// Returns the property.
    #[must_use]
    pub fn property(&self) -> &str {
        &self.property
    }

    /// Inserts an edge into the index.
    pub fn insert(&mut self, edge_id: u64, value: &Value) {
        self.index
            .entry(OrderedValue(value.clone()))
            .or_default()
            .push(edge_id);
    }

    /// Removes an edge from the index.
    pub fn remove(&mut self, edge_id: u64, value: &Value) -> bool {
        let key = OrderedValue(value.clone());
        if let Some(edges) = self.index.get_mut(&key) {
            if let Some(pos) = edges.iter().position(|&id| id == edge_id) {
                edges.swap_remove(pos);
                if edges.is_empty() {
                    self.index.remove(&key);
                }
                return true;
            }
        }
        false
    }

    /// Looks up edges by exact value.
    #[must_use]
    pub fn lookup_exact(&self, value: &Value) -> &[u64] {
        self.index
            .get(&OrderedValue(value.clone()))
            .map_or(&[], Vec::as_slice)
    }

    /// Range lookup for edges.
    pub fn lookup_range(&self, lower: Option<&Value>, upper: Option<&Value>) -> Vec<u64> {
        use std::ops::Bound;

        let start = match lower {
            Some(v) => Bound::Included(OrderedValue(v.clone())),
            None => Bound::Unbounded,
        };

        let end = match upper {
            Some(v) => Bound::Included(OrderedValue(v.clone())),
            None => Bound::Unbounded,
        };

        self.index
            .range((start, end))
            .flat_map(|(_, ids)| ids.iter().copied())
            .collect()
    }
}

// =============================================================================
// EPIC-047 US-004: Index Intersection
// =============================================================================

/// Utilities for intersecting index results.
#[allow(dead_code)]
pub struct IndexIntersection;

#[allow(dead_code)]
impl IndexIntersection {
    /// Intersects multiple node ID sets using RoaringBitmap for efficiency.
    #[must_use]
    pub fn intersect_bitmaps(sets: &[RoaringBitmap]) -> RoaringBitmap {
        if sets.is_empty() {
            return RoaringBitmap::new();
        }

        let mut result = sets[0].clone();
        for set in &sets[1..] {
            result &= set;
            // Early exit if empty
            if result.is_empty() {
                return result;
            }
        }
        result
    }

    /// Intersects multiple Vec<u64> sets, converting to bitmaps.
    ///
    /// # Warning
    ///
    /// IDs greater than `u32::MAX` will be dropped and logged as a warning,
    /// since `RoaringBitmap` only supports 32-bit integers.
    #[must_use]
    pub fn intersect_vecs(sets: &[&[u64]]) -> Vec<u64> {
        if sets.is_empty() {
            return Vec::new();
        }

        // BUG-2 FIX: Log warning when IDs > u32::MAX are dropped
        let mut dropped_count = 0usize;
        let bitmaps: Vec<RoaringBitmap> = sets
            .iter()
            .map(|s| {
                s.iter()
                    .filter_map(|&id| {
                        if let Ok(id32) = u32::try_from(id) {
                            Some(id32)
                        } else {
                            dropped_count += 1;
                            None
                        }
                    })
                    .collect()
            })
            .collect();

        if dropped_count > 0 {
            tracing::warn!(
                dropped_count,
                "intersect_vecs: {} IDs > u32::MAX were silently dropped. \
                 Consider using intersect_two() for large ID ranges.",
                dropped_count
            );
        }

        Self::intersect_bitmaps(&bitmaps)
            .iter()
            .map(u64::from)
            .collect()
    }

    /// Intersects two sets with early exit optimization.
    #[must_use]
    pub fn intersect_two(a: &[u64], b: &[u64]) -> Vec<u64> {
        // Use the smaller set for lookup
        let (smaller, larger) = if a.len() < b.len() { (a, b) } else { (b, a) };

        let larger_set: std::collections::HashSet<_> = larger.iter().collect();
        smaller
            .iter()
            .filter(|id| larger_set.contains(id))
            .copied()
            .collect()
    }
}
