//! Range indexes and edge property indexes (EPIC-047 US-002, US-003).
//!
//! Provides B-tree based range indexes for ordered queries on node and edge properties.
//!
//! `CompositeRangeIndex` and `EdgePropertyIndex` share the same underlying
//! B-tree storage via [`BTreeOrderedIndex`], differing only in their metadata
//! (label vs relationship-type) and public API surface.

use roaring::RoaringBitmap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::ops::Bound;

// =============================================================================
// OrderedValue: total ordering wrapper for JSON values
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

// =============================================================================
// BTreeOrderedIndex: shared B-tree storage backing both index types
// =============================================================================

/// Shared B-tree storage for ordered property indexes.
///
/// Both [`CompositeRangeIndex`] (node properties) and [`EdgePropertyIndex`]
/// (edge properties) delegate to this type for all insert/remove/lookup
/// operations, eliminating code duplication.
#[allow(dead_code)]
#[derive(Debug, Serialize, Deserialize)]
struct BTreeOrderedIndex {
    entries: std::collections::BTreeMap<OrderedValue, Vec<u64>>,
}

#[allow(dead_code)]
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
#[allow(dead_code)]
#[derive(Debug, Serialize, Deserialize)]
pub struct CompositeRangeIndex {
    /// Label this index covers
    label: String,
    /// Property name
    property: String,
    /// Shared B-tree storage
    index: BTreeOrderedIndex,
}

#[allow(dead_code)]
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
#[allow(dead_code)]
#[derive(Debug, Serialize, Deserialize)]
pub struct EdgePropertyIndex {
    /// Relationship type this index covers
    rel_type: String,
    /// Property name
    property: String,
    /// Shared B-tree storage
    index: BTreeOrderedIndex,
}

#[allow(dead_code)]
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
        self.index.insert(edge_id, value);
    }

    /// Removes an edge from the index.
    pub fn remove(&mut self, edge_id: u64, value: &Value) -> bool {
        self.index.remove(edge_id, value)
    }

    /// Looks up edges by exact value.
    #[must_use]
    pub fn lookup_exact(&self, value: &Value) -> &[u64] {
        self.index.lookup_exact(value)
    }

    /// Range lookup for edges.
    pub fn lookup_range(&self, lower: Option<&Value>, upper: Option<&Value>) -> Vec<u64> {
        self.index.lookup_range(lower, upper)
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
