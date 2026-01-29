//! Property index for fast graph node lookups.
//!
//! Provides O(1) lookups on (label, property_name, value) instead of O(n) scans.
//! Also includes composite indexes for (label, property1, property2, ...) lookups.

use roaring::RoaringBitmap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

/// Current schema version for PropertyIndex serialization.
/// Increment this when making breaking changes to the index format.
pub const PROPERTY_INDEX_VERSION: u32 = 1;

/// Index for fast property-based node lookups.
///
/// Maps (label, property_name) -> (value -> node_ids) for O(1) lookups.
///
/// # Example
///
/// ```rust,ignore
/// let mut index = PropertyIndex::new();
/// index.create_index("Person", "email");
/// index.insert("Person", "email", &json!("alice@example.com"), 1);
///
/// let nodes = index.lookup("Person", "email", &json!("alice@example.com"));
/// assert!(nodes.map_or(false, |b| b.contains(1)));
/// ```
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct PropertyIndex {
    /// Schema version for forward compatibility.
    #[serde(default = "default_version")]
    version: u32,
    /// (label, property_name) -> (value_json -> node_ids)
    indexes: HashMap<(String, String), HashMap<String, RoaringBitmap>>,
}

fn default_version() -> u32 {
    PROPERTY_INDEX_VERSION
}

impl PropertyIndex {
    /// Create a new empty property index.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Create an index for a (label, property) pair.
    ///
    /// This must be called before inserting values for this pair.
    pub fn create_index(&mut self, label: &str, property: &str) {
        let key = (label.to_string(), property.to_string());
        self.indexes.entry(key).or_default();
    }

    /// Check if an index exists for this (label, property) pair.
    #[must_use]
    pub fn has_index(&self, label: &str, property: &str) -> bool {
        self.indexes
            .keys()
            .any(|(l, p)| l == label && p == property)
    }

    /// Insert a node into the index.
    ///
    /// Returns `true` if the index exists and the node was added.
    ///
    /// # Note
    ///
    /// Uses RoaringBitmap internally which only supports u32 IDs.
    /// Returns `false` if node_id > u32::MAX to prevent data corruption.
    pub fn insert(&mut self, label: &str, property: &str, value: &Value, node_id: u64) -> bool {
        // BUG FIX: Reject node_id > u32::MAX instead of silently truncating
        // This prevents data corruption from ID collisions
        let Some(safe_id) = u32::try_from(node_id).ok() else {
            tracing::warn!(
                node_id = node_id,
                label = label,
                property = property,
                "PropertyIndex: node_id exceeds u32::MAX ({}), cannot index. \
                 RoaringBitmap only supports u32 IDs.",
                u32::MAX
            );
            return false;
        };

        let key = (label.to_string(), property.to_string());
        if let Some(value_map) = self.indexes.get_mut(&key) {
            let value_key = value.to_string();
            value_map.entry(value_key).or_default().insert(safe_id);
            true
        } else {
            false
        }
    }

    /// Remove a node from the index.
    ///
    /// Returns `true` if the node was removed.
    /// Returns `false` if node_id > u32::MAX (cannot exist in index).
    pub fn remove(&mut self, label: &str, property: &str, value: &Value, node_id: u64) -> bool {
        // BUG FIX: node_id > u32::MAX cannot exist in index, return false
        let Some(safe_id) = u32::try_from(node_id).ok() else {
            return false;
        };

        let key = (label.to_string(), property.to_string());
        if let Some(value_map) = self.indexes.get_mut(&key) {
            let value_key = value.to_string();
            if let Some(bitmap) = value_map.get_mut(&value_key) {
                let removed = bitmap.remove(safe_id);
                if bitmap.is_empty() {
                    value_map.remove(&value_key);
                }
                return removed;
            }
        }
        false
    }

    /// Lookup nodes by property value.
    ///
    /// Returns `None` if no index exists for this (label, property) pair.
    /// Returns `Some(&RoaringBitmap)` with matching node IDs (empty if no matches).
    #[must_use]
    pub fn lookup(&self, label: &str, property: &str, value: &Value) -> Option<&RoaringBitmap> {
        self.indexes
            .iter()
            .find(|((l, p), _)| l == label && p == property)
            .and_then(|(_, value_map)| {
                let value_key = value.to_string();
                value_map.get(&value_key)
            })
    }

    /// Get all indexed (label, property) pairs.
    #[must_use]
    pub fn indexed_properties(&self) -> Vec<(String, String)> {
        self.indexes.keys().cloned().collect()
    }

    /// Get the number of unique values for a (label, property) pair.
    #[must_use]
    pub fn cardinality(&self, label: &str, property: &str) -> Option<usize> {
        self.indexes
            .iter()
            .find(|((l, p), _)| l == label && p == property)
            .map(|(_, value_map)| value_map.len())
    }

    /// Drop an index for a (label, property) pair.
    pub fn drop_index(&mut self, label: &str, property: &str) -> bool {
        let key = (label.to_string(), property.to_string());
        self.indexes.remove(&key).is_some()
    }

    /// Clear all indexes.
    pub fn clear(&mut self) {
        self.indexes.clear();
    }

    /// Get total memory estimate in bytes.
    #[must_use]
    pub fn memory_usage(&self) -> usize {
        let mut total = std::mem::size_of::<Self>();
        for ((label, prop), value_map) in &self.indexes {
            total += label.len() + prop.len();
            for (value_key, bitmap) in value_map {
                total += value_key.len();
                total += bitmap.serialized_size();
            }
        }
        total
    }

    // =========================================================================
    // Maintenance hooks - called automatically on graph mutations
    // =========================================================================

    /// Hook called when a node is added to the graph.
    ///
    /// Indexes all properties that have an active index.
    pub fn on_add_node(&mut self, label: &str, node_id: u64, properties: &HashMap<String, Value>) {
        for (prop_name, value) in properties {
            if self.has_index(label, prop_name) {
                self.insert(label, prop_name, value, node_id);
            }
        }
    }

    /// Hook called when a node is removed from the graph.
    ///
    /// Removes all indexed properties for this node.
    pub fn on_remove_node(
        &mut self,
        label: &str,
        node_id: u64,
        properties: &HashMap<String, Value>,
    ) {
        for (prop_name, value) in properties {
            if self.has_index(label, prop_name) {
                self.remove(label, prop_name, value, node_id);
            }
        }
    }

    /// Hook called when a property is updated on a node.
    ///
    /// Removes old value and inserts new value if property is indexed.
    pub fn on_update_property(
        &mut self,
        label: &str,
        node_id: u64,
        property: &str,
        old_value: &Value,
        new_value: &Value,
    ) {
        if self.has_index(label, property) {
            self.remove(label, property, old_value, node_id);
            self.insert(label, property, new_value, node_id);
        }
    }

    /// Hook to index all properties of a node after creating an index.
    ///
    /// Use this to backfill an index after creation.
    pub fn index_node(&mut self, label: &str, node_id: u64, properties: &HashMap<String, Value>) {
        self.on_add_node(label, node_id, properties);
    }

    // =========================================================================
    // Persistence - serialize/deserialize index to/from bytes
    // =========================================================================

    /// Serialize the index to bytes using bincode.
    ///
    /// # Errors
    /// Returns an error if serialization fails.
    pub fn to_bytes(&self) -> Result<Vec<u8>, bincode::Error> {
        bincode::serialize(self)
    }

    /// Deserialize an index from bytes.
    ///
    /// # Errors
    /// Returns an error if deserialization fails (corrupted data).
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, bincode::Error> {
        bincode::deserialize(bytes)
    }

    /// Save the index to a file.
    ///
    /// # Errors
    /// Returns an error if serialization or file I/O fails.
    pub fn save_to_file(&self, path: &std::path::Path) -> std::io::Result<()> {
        let bytes = self
            .to_bytes()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
        std::fs::write(path, bytes)
    }

    /// Load an index from a file.
    ///
    /// # Errors
    /// Returns an error if file I/O or deserialization fails.
    pub fn load_from_file(path: &std::path::Path) -> std::io::Result<Self> {
        let bytes = std::fs::read(path)?;
        Self::from_bytes(&bytes)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))
    }
}

// =============================================================================
// EPIC-047 US-001: Composite Graph Index
// =============================================================================

/// Index type for composite indexes.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompositeIndexType {
    /// Hash index for equality lookups (O(1))
    Hash,
    /// Range index for range queries (O(log n))
    Range,
}

/// Composite index on (label, property1, property2, ...).
///
/// Provides O(1) lookups for nodes matching a label and specific property values.
/// Useful for queries like `MATCH (n:Person {name: 'Alice', city: 'Paris'})`.
///
/// # Example
///
/// ```rust,ignore
/// let mut index = CompositeGraphIndex::new("Person", vec!["name", "city"], CompositeIndexType::Hash);
/// index.insert(1, &[json!("Alice"), json!("Paris")]);
/// index.insert(2, &[json!("Bob"), json!("London")]);
///
/// let nodes = index.lookup(&[json!("Alice"), json!("Paris")]);
/// assert_eq!(nodes, &[1]);
/// ```
#[allow(dead_code)]
#[derive(Debug, Serialize, Deserialize)]
pub struct CompositeGraphIndex {
    /// Label this index covers
    label: String,
    /// Property names in order
    properties: Vec<String>,
    /// Index type (hash or range)
    index_type: CompositeIndexType,
    /// (property_values_hash) -> Vec<NodeId>
    hash_index: HashMap<u64, Vec<u64>>,
}

#[allow(dead_code)]
impl CompositeGraphIndex {
    /// Creates a new composite index.
    #[must_use]
    pub fn new(
        label: impl Into<String>,
        properties: Vec<String>,
        index_type: CompositeIndexType,
    ) -> Self {
        Self {
            label: label.into(),
            properties,
            index_type,
            hash_index: HashMap::new(),
        }
    }

    /// Returns the label this index covers.
    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Returns the properties this index covers.
    #[must_use]
    pub fn properties(&self) -> &[String] {
        &self.properties
    }

    /// Returns the index type.
    #[must_use]
    pub fn index_type(&self) -> CompositeIndexType {
        self.index_type
    }

    /// Checks if this index covers the given label and properties.
    #[must_use]
    pub fn covers(&self, label: &str, properties: &[&str]) -> bool {
        if self.label != label {
            return false;
        }
        // Check if all requested properties are covered by this index
        properties
            .iter()
            .all(|p| self.properties.iter().any(|ip| ip == *p))
    }

    /// Inserts a node into the index.
    pub fn insert(&mut self, node_id: u64, values: &[Value]) {
        if values.len() != self.properties.len() {
            tracing::warn!(
                "CompositeGraphIndex: value count ({}) != property count ({})",
                values.len(),
                self.properties.len()
            );
            return;
        }

        let hash = Self::hash_values(values);
        self.hash_index.entry(hash).or_default().push(node_id);
    }

    /// Removes a node from the index.
    pub fn remove(&mut self, node_id: u64, values: &[Value]) -> bool {
        if values.len() != self.properties.len() {
            return false;
        }

        let hash = Self::hash_values(values);
        if let Some(nodes) = self.hash_index.get_mut(&hash) {
            if let Some(pos) = nodes.iter().position(|&id| id == node_id) {
                nodes.swap_remove(pos);
                if nodes.is_empty() {
                    self.hash_index.remove(&hash);
                }
                return true;
            }
        }
        false
    }

    /// Looks up nodes by property values.
    #[must_use]
    pub fn lookup(&self, values: &[Value]) -> &[u64] {
        if values.len() != self.properties.len() {
            return &[];
        }

        let hash = Self::hash_values(values);
        self.hash_index.get(&hash).map_or(&[], Vec::as_slice)
    }

    /// Returns the number of unique value combinations in the index.
    #[must_use]
    pub fn cardinality(&self) -> usize {
        self.hash_index.len()
    }

    /// Returns the total number of indexed nodes.
    #[must_use]
    pub fn node_count(&self) -> usize {
        self.hash_index.values().map(Vec::len).sum()
    }

    /// Clears the index.
    pub fn clear(&mut self) {
        self.hash_index.clear();
    }

    /// Computes a hash of the property values.
    fn hash_values(values: &[Value]) -> u64 {
        let mut hasher = DefaultHasher::new();
        for value in values {
            // Hash the JSON string representation for consistency
            value.to_string().hash(&mut hasher);
        }
        hasher.finish()
    }

    /// Gets the memory usage estimate in bytes.
    #[must_use]
    pub fn memory_usage(&self) -> usize {
        let mut total = std::mem::size_of::<Self>();
        total += self.label.len();
        for prop in &self.properties {
            total += prop.len();
        }
        // Each hash entry: u64 key + Vec overhead + node IDs
        for nodes in self.hash_index.values() {
            total += 8 + std::mem::size_of::<Vec<u64>>() + nodes.len() * 8;
        }
        total
    }
}

/// Manager for multiple composite indexes.
#[allow(dead_code)]
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct CompositeIndexManager {
    /// All composite indexes, keyed by index name
    indexes: HashMap<String, CompositeGraphIndex>,
}

#[allow(dead_code)]
impl CompositeIndexManager {
    /// Creates a new index manager.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a new composite index.
    pub fn create_index(
        &mut self,
        name: impl Into<String>,
        label: impl Into<String>,
        properties: Vec<String>,
        index_type: CompositeIndexType,
    ) -> bool {
        let name = name.into();
        if self.indexes.contains_key(&name) {
            return false;
        }
        let index = CompositeGraphIndex::new(label, properties, index_type);
        self.indexes.insert(name, index);
        true
    }

    /// Gets an index by name.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&CompositeGraphIndex> {
        self.indexes.get(name)
    }

    /// Gets a mutable index by name.
    pub fn get_mut(&mut self, name: &str) -> Option<&mut CompositeGraphIndex> {
        self.indexes.get_mut(name)
    }

    /// Drops an index by name.
    pub fn drop_index(&mut self, name: &str) -> bool {
        self.indexes.remove(name).is_some()
    }

    /// Finds indexes that cover the given label and properties.
    #[must_use]
    pub fn find_covering_indexes(&self, label: &str, properties: &[&str]) -> Vec<&str> {
        self.indexes
            .iter()
            .filter(|(_, idx)| idx.covers(label, properties))
            .map(|(name, _)| name.as_str())
            .collect()
    }

    /// Lists all index names.
    #[must_use]
    pub fn list_indexes(&self) -> Vec<&str> {
        self.indexes.keys().map(String::as_str).collect()
    }

    /// Updates all indexes when a node is added.
    pub fn on_add_node(&mut self, label: &str, node_id: u64, properties: &HashMap<String, Value>) {
        for index in self.indexes.values_mut() {
            if index.label() == label {
                // Extract values in the order of index properties
                let values: Vec<Value> = index
                    .properties()
                    .iter()
                    .map(|p| properties.get(p).cloned().unwrap_or(Value::Null))
                    .collect();
                index.insert(node_id, &values);
            }
        }
    }

    /// Updates all indexes when a node is removed.
    pub fn on_remove_node(
        &mut self,
        label: &str,
        node_id: u64,
        properties: &HashMap<String, Value>,
    ) {
        for index in self.indexes.values_mut() {
            if index.label() == label {
                let values: Vec<Value> = index
                    .properties()
                    .iter()
                    .map(|p| properties.get(p).cloned().unwrap_or(Value::Null))
                    .collect();
                index.remove(node_id, &values);
            }
        }
    }
}

// =============================================================================
// EPIC-047 US-002: Range Index (B-tree based)
// =============================================================================

/// Wrapper for total ordering on JSON values.
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderedValue(Value);

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
                    .filter_map(|&id| match u32::try_from(id) {
                        Ok(id32) => Some(id32),
                        Err(_) => {
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

// =============================================================================
// EPIC-047 US-005: Auto-Index Suggestions
// =============================================================================

/// Predicate types for query pattern tracking.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PredicateType {
    /// Equality check (=)
    Equality,
    /// Range comparison (>, <, >=, <=)
    Range,
    /// IN list
    In,
    /// LIKE pattern
    Like,
}

/// A query pattern for index suggestion analysis.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct QueryPattern {
    /// Labels involved
    pub labels: Vec<String>,
    /// Properties filtered on
    pub properties: Vec<String>,
    /// Types of predicates used
    pub predicates: Vec<PredicateType>,
}

/// Statistics for a query pattern.
#[allow(dead_code)]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PatternStats {
    /// Number of times this pattern was seen
    pub count: u64,
    /// Total execution time in milliseconds
    pub total_time_ms: u64,
    /// Average execution time
    pub avg_time_ms: f64,
    /// Last seen timestamp (unix millis)
    pub last_seen_ms: u64,
}

/// Tracks query patterns for index suggestion.
#[allow(dead_code)]
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct QueryPatternTracker {
    /// Pattern -> stats mapping
    patterns: HashMap<QueryPattern, PatternStats>,
    /// Threshold for slow query (ms)
    slow_query_threshold_ms: u64,
}

#[allow(dead_code)]
impl QueryPatternTracker {
    /// Creates a new tracker with default threshold (100ms).
    #[must_use]
    pub fn new() -> Self {
        Self {
            patterns: HashMap::new(),
            slow_query_threshold_ms: 100,
        }
    }

    /// Sets the slow query threshold.
    pub fn set_threshold(&mut self, threshold_ms: u64) {
        self.slow_query_threshold_ms = threshold_ms;
    }

    /// Records a query execution.
    pub fn record(&mut self, pattern: QueryPattern, execution_time_ms: u64) {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        let stats = self.patterns.entry(pattern).or_default();
        stats.count += 1;
        stats.total_time_ms += execution_time_ms;
        #[allow(clippy::cast_precision_loss)]
        {
            stats.avg_time_ms = stats.total_time_ms as f64 / stats.count as f64;
        }
        stats.last_seen_ms = now_ms;
    }

    /// Returns patterns sorted by total time (most expensive first).
    #[must_use]
    pub fn expensive_patterns(&self) -> Vec<(&QueryPattern, &PatternStats)> {
        let mut patterns: Vec<_> = self.patterns.iter().collect();
        patterns.sort_by(|a, b| b.1.total_time_ms.cmp(&a.1.total_time_ms));
        patterns
    }

    /// Returns patterns that are slow (above threshold).
    #[must_use]
    pub fn slow_patterns(&self) -> Vec<(&QueryPattern, &PatternStats)> {
        self.patterns
            .iter()
            .filter(|(_, stats)| stats.avg_time_ms > self.slow_query_threshold_ms as f64)
            .collect()
    }
}

/// An index suggestion.
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexSuggestion {
    /// DDL statement to create the index
    pub ddl: String,
    /// The pattern this would help
    pub pattern: QueryPattern,
    /// Estimated improvement (0.0 to 1.0)
    pub estimated_improvement: f64,
    /// Number of queries that would benefit
    pub query_count: u64,
    /// Priority score (higher = more important)
    pub priority_score: f64,
}

/// Advisor that suggests indexes based on query patterns.
#[allow(dead_code)]
#[derive(Debug, Default)]
pub struct IndexAdvisor {
    /// Existing index names (to avoid duplicates)
    existing_indexes: std::collections::HashSet<String>,
}

#[allow(dead_code)]
impl IndexAdvisor {
    /// Creates a new advisor.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers an existing index.
    pub fn register_index(&mut self, name: impl Into<String>) {
        self.existing_indexes.insert(name.into());
    }

    /// Generates suggestions from tracked patterns.
    #[must_use]
    pub fn suggest(&self, tracker: &QueryPatternTracker) -> Vec<IndexSuggestion> {
        let mut suggestions = Vec::new();

        for (pattern, stats) in tracker.expensive_patterns() {
            // Skip if no properties to index
            if pattern.properties.is_empty() || pattern.labels.is_empty() {
                continue;
            }

            let index_name = format!(
                "idx_{}_{}",
                pattern.labels.join("_").to_lowercase(),
                pattern.properties.join("_").to_lowercase()
            );

            // Skip if index already exists
            if self.existing_indexes.contains(&index_name) {
                continue;
            }

            // Estimate improvement based on predicate type
            let improvement = Self::estimate_improvement(pattern);
            if improvement < 0.2 {
                continue;
            }

            // Calculate priority: frequency * improvement * avg_time
            let priority = stats.count as f64 * improvement * stats.avg_time_ms;

            let ddl = format!(
                "CREATE INDEX {} ON :{}({})",
                index_name,
                pattern.labels.first().unwrap_or(&String::new()),
                pattern.properties.join(", ")
            );

            suggestions.push(IndexSuggestion {
                ddl,
                pattern: pattern.clone(),
                estimated_improvement: improvement,
                query_count: stats.count,
                priority_score: priority,
            });
        }

        // Sort by priority (highest first)
        suggestions.sort_by(|a, b| {
            b.priority_score
                .partial_cmp(&a.priority_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        suggestions
    }

    /// Estimates improvement from adding an index.
    fn estimate_improvement(pattern: &QueryPattern) -> f64 {
        let mut improvement = 0.0;

        for pred in &pattern.predicates {
            match pred {
                PredicateType::Equality => improvement += 0.9,
                PredicateType::Range => improvement += 0.7,
                PredicateType::In => improvement += 0.6,
                PredicateType::Like => improvement += 0.3,
            }
        }

        // Normalize to 0.0-1.0
        (improvement / pattern.predicates.len().max(1) as f64).min(1.0)
    }
}
