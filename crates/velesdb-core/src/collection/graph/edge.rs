//! Graph edge types and storage for knowledge graph relationships.
//!
//! This module provides:
//! - `GraphEdge`: A typed relationship between nodes with properties
//! - `EdgeStore`: Bidirectional index for efficient edge traversal
//! - `CsrSnapshot`: Zero-copy CSR snapshot for cache-friendly BFS traversal
//!
//! # Edge Removal Semantics
//!
//! During edge removal, the internal indexes may be temporarily inconsistent
//! while the operation is in progress. The final state is always consistent.
//! For concurrent access, use `ConcurrentEdgeStore` instead.

use super::helpers::PostcardPersistence;
use crate::error::{Error, Result};
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// A directed edge (relationship) in the knowledge graph.
///
/// Edges connect nodes and can have a label (type) and properties.
///
/// # Example
///
/// ```rust,ignore
/// use velesdb_core::collection::graph::GraphEdge;
/// use serde_json::json;
/// use std::collections::HashMap;
///
/// let mut props = HashMap::new();
/// props.insert("since".to_string(), json!("2020-01-01"));
///
/// let edge = GraphEdge::new(1, 100, 200, "KNOWS")
///     .with_properties(props);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GraphEdge {
    id: u64,
    source: u64,
    target: u64,
    label: String,
    properties: HashMap<String, Value>,
}

impl GraphEdge {
    /// Creates a new edge with the given ID, endpoints, and label.
    ///
    /// # Errors
    ///
    /// Returns `Error::InvalidEdgeLabel` if the label is empty or whitespace-only.
    pub fn new(id: u64, source: u64, target: u64, label: &str) -> Result<Self> {
        let trimmed = label.trim();
        if trimmed.is_empty() {
            return Err(Error::InvalidEdgeLabel(
                "Edge label cannot be empty or whitespace-only".to_string(),
            ));
        }
        Ok(Self {
            id,
            source,
            target,
            label: trimmed.to_string(),
            properties: HashMap::new(),
        })
    }

    /// Adds properties to this edge (builder pattern).
    #[must_use]
    pub fn with_properties(mut self, properties: HashMap<String, Value>) -> Self {
        self.properties = properties;
        self
    }

    /// Returns the edge ID.
    #[must_use]
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Returns the source node ID.
    #[must_use]
    pub fn source(&self) -> u64 {
        self.source
    }

    /// Returns the target node ID.
    #[must_use]
    pub fn target(&self) -> u64 {
        self.target
    }

    /// Returns the edge label (relationship type).
    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Returns all properties of this edge.
    #[must_use]
    pub fn properties(&self) -> &HashMap<String, Value> {
        &self.properties
    }

    /// Returns a specific property value, if it exists.
    #[must_use]
    pub fn property(&self, name: &str) -> Option<&Value> {
        self.properties.get(name)
    }
}

/// Compressed Sparse Row (CSR) snapshot for zero-copy BFS traversal.
///
/// Stores outgoing neighbor target IDs and their corresponding edge IDs
/// in two parallel contiguous arrays, indexed by `(offset, len)` per source node.
/// This eliminates per-edge `GraphEdge` cloning during BFS frontier expansion.
///
/// # Memory layout
///
/// ```text
/// index: { node_1 -> (0, 3), node_2 -> (3, 2), ... }
/// targets: [t1, t2, t3, t4, t5, ...]
/// edge_ids: [e1, e2, e3, e4, e5, ...]
/// ```
///
/// `targets[offset..offset+len]` are the neighbor target IDs for a source node,
/// and `edge_ids[offset..offset+len]` are the corresponding edge IDs.
#[derive(Debug, Clone)]
pub struct CsrSnapshot {
    /// Maps source_node_id -> (offset, length) into `targets`/`edge_ids`.
    index: FxHashMap<u64, (usize, usize)>,
    /// Contiguous storage of target node IDs for all outgoing edges.
    targets: Vec<u64>,
    /// Contiguous storage of edge IDs, parallel to `targets`.
    edge_ids: Vec<u64>,
    /// Contiguous storage of label indices, parallel to `targets`.
    /// Each label index maps to a string in `label_table`.
    label_indices: Vec<u32>,
    /// Interned label strings for memory-efficient rel-type filtering.
    label_table: Vec<String>,
    /// Reverse map: label string -> label index for O(1) lookup.
    label_to_idx: FxHashMap<String, u32>,
}

impl CsrSnapshot {
    /// Returns neighbor target IDs for a source node as a zero-copy slice.
    #[must_use]
    #[inline]
    pub fn neighbors(&self, source_id: u64) -> &[u64] {
        if let Some(&(offset, len)) = self.index.get(&source_id) {
            &self.targets[offset..offset + len]
        } else {
            &[]
        }
    }

    /// Returns edge IDs for a source node as a zero-copy slice.
    ///
    /// Parallel to `neighbors()`: `edge_ids[i]` is the edge connecting
    /// `source_id` to `neighbors[i]`.
    #[must_use]
    #[inline]
    pub fn edge_ids(&self, source_id: u64) -> &[u64] {
        if let Some(&(offset, len)) = self.index.get(&source_id) {
            &self.edge_ids[offset..offset + len]
        } else {
            &[]
        }
    }

    /// Returns the label string for a neighbor at position `idx` relative
    /// to the source's offset.
    ///
    /// Returns `None` if `source_id` has no CSR entry or `neighbor_idx`
    /// is out of range.
    #[must_use]
    #[inline]
    pub fn label_at(&self, source_id: u64, neighbor_idx: usize) -> Option<&str> {
        let &(offset, len) = self.index.get(&source_id)?;
        if neighbor_idx >= len {
            return None;
        }
        let label_idx = self.label_indices[offset + neighbor_idx] as usize;
        self.label_table.get(label_idx).map(String::as_str)
    }

    /// Checks whether a label string exists in the interned table.
    ///
    /// Used for fast pre-filtering: if a rel-type filter contains labels
    /// not present in the snapshot, those branches can be skipped entirely.
    #[must_use]
    #[inline]
    pub fn has_label(&self, label: &str) -> bool {
        self.label_to_idx.contains_key(label)
    }
}

/// Storage for graph edges with bidirectional indexing.
///
/// Provides O(1) access to edges by ID and O(degree) access to
/// outgoing/incoming edges for any node.
///
/// # Index Structure (EPIC-019 US-003)
///
/// - `by_label`: Secondary index for O(k) label-based queries
/// - `outgoing_by_label`: Composite index (source, label) for O(k) filtered traversal
///
/// # CSR Snapshot (G1)
///
/// After loading from disk or after explicit `build_read_snapshot()`, the
/// `csr_snapshot` field provides zero-copy `&[u64]` access to neighbor
/// target IDs and edge IDs. Writes invalidate the snapshot automatically.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct EdgeStore {
    /// All edges indexed by ID
    edges: HashMap<u64, GraphEdge>,
    /// Outgoing edges: source_id -> Vec<edge_id>
    outgoing: HashMap<u64, Vec<u64>>,
    /// Incoming edges: target_id -> Vec<edge_id>
    incoming: HashMap<u64, Vec<u64>>,
    /// Secondary index: label -> Vec<edge_id> for fast label queries
    by_label: HashMap<String, Vec<u64>>,
    /// Composite index: (source_id, label) -> Vec<edge_id> for fast filtered traversal
    outgoing_by_label: HashMap<(u64, String), Vec<u64>>,
    /// Zero-copy CSR snapshot for BFS traversal (G1).
    /// Built on-demand via `build_read_snapshot()`, invalidated by writes.
    #[serde(skip)]
    csr_snapshot: Option<CsrSnapshot>,
}

impl EdgeStore {
    /// Creates a new empty edge store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates an edge store with pre-allocated capacity for better performance.
    ///
    /// Pre-allocating reduces memory reallocation overhead when inserting many edges.
    /// With 10M edges, this can reduce peak memory usage by ~2x and improve insert throughput.
    ///
    /// # Arguments
    ///
    /// * `expected_edges` - Expected number of edges to store
    /// * `expected_nodes` - Expected number of unique nodes (sources + targets)
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// // For a graph with ~1M edges and ~100K nodes
    /// let store = EdgeStore::with_capacity(1_000_000, 100_000);
    /// ```
    #[must_use]
    pub fn with_capacity(expected_edges: usize, expected_nodes: usize) -> Self {
        // Estimate ~10 unique labels typical for knowledge graphs
        let expected_labels = 10usize;
        // Use saturating_mul to prevent overflow for extreme inputs
        let outgoing_by_label_cap = expected_nodes
            .saturating_mul(expected_labels)
            .saturating_div(10);
        Self {
            edges: HashMap::with_capacity(expected_edges),
            outgoing: HashMap::with_capacity(expected_nodes),
            incoming: HashMap::with_capacity(expected_nodes),
            by_label: HashMap::with_capacity(expected_labels),
            outgoing_by_label: HashMap::with_capacity(outgoing_by_label_cap),
            csr_snapshot: None,
        }
    }

    /// Adds an edge to the store.
    ///
    /// Creates bidirectional index entries for efficient traversal.
    /// Also maintains label-based secondary indices (EPIC-019 US-003).
    ///
    /// # Errors
    ///
    /// Returns `Error::EdgeExists` if an edge with the same ID already exists.
    pub fn add_edge(&mut self, edge: GraphEdge) -> Result<()> {
        self.insert_edge(edge, true, true)
    }

    /// Adds an edge with only the outgoing index (for cross-shard storage).
    ///
    /// Used by `ConcurrentEdgeStore` when source and target are in different shards.
    /// The edge is stored and indexed by source node only.
    ///
    /// # Errors
    ///
    /// Returns `Error::EdgeExists` if an edge with the same ID already exists.
    pub fn add_edge_outgoing_only(&mut self, edge: GraphEdge) -> Result<()> {
        self.insert_edge(edge, true, false)
    }

    /// Adds an edge with only the incoming index (for cross-shard storage).
    ///
    /// Used by `ConcurrentEdgeStore` when source and target are in different shards.
    /// The edge is stored and indexed by target node only.
    /// Note: Label indices are maintained by the source shard in `ConcurrentEdgeStore`.
    ///
    /// # Errors
    ///
    /// Returns `Error::EdgeExists` if an edge with the same ID already exists.
    pub fn add_edge_incoming_only(&mut self, edge: GraphEdge) -> Result<()> {
        self.insert_edge(edge, false, true)
    }

    /// Shared implementation for all `add_edge*` variants.
    ///
    /// Validates uniqueness, populates the requested directional indices,
    /// and stores the edge. Label indices (`by_label`, `outgoing_by_label`)
    /// are maintained only when `index_outgoing` is `true` (source shard
    /// owns label indices in the concurrent model).
    fn insert_edge(
        &mut self,
        edge: GraphEdge,
        index_outgoing: bool,
        index_incoming: bool,
    ) -> Result<()> {
        let id = edge.id();
        if self.edges.contains_key(&id) {
            return Err(Error::EdgeExists(id));
        }

        if index_outgoing {
            let source = edge.source();
            let label = edge.label().to_string();
            self.outgoing.entry(source).or_default().push(id);
            // Label indices are owned by the source shard (US-003)
            self.by_label.entry(label.clone()).or_default().push(id);
            self.outgoing_by_label
                .entry((source, label))
                .or_default()
                .push(id);
        }

        if index_incoming {
            self.incoming.entry(edge.target()).or_default().push(id);
        }

        self.edges.insert(id, edge);
        // Invalidate CSR snapshot — writes make it stale (G1).
        self.csr_snapshot = None;
        Ok(())
    }

    /// Returns the total number of edges in the store.
    #[must_use]
    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    /// Returns the count of edges where this shard is the source (for accurate cross-shard counting).
    #[must_use]
    pub fn outgoing_edge_count(&self) -> usize {
        self.outgoing.values().map(Vec::len).sum()
    }

    /// Gets an edge by its ID.
    #[must_use]
    pub fn get_edge(&self, id: u64) -> Option<&GraphEdge> {
        self.edges.get(&id)
    }

    /// Gets all outgoing edges from a node.
    #[must_use]
    pub fn get_outgoing(&self, node_id: u64) -> Vec<&GraphEdge> {
        self.resolve_edge_ids(self.outgoing.get(&node_id))
    }

    /// Invokes `f` for each outgoing edge from `node_id` without allocating a `Vec`.
    ///
    /// Prefer this over [`get_outgoing`](Self::get_outgoing) in hot loops (e.g. BFS
    /// frontiers) where the caller processes edges inline rather than collecting them.
    #[inline]
    pub fn for_each_outgoing<F: FnMut(&GraphEdge)>(&self, node_id: u64, mut f: F) {
        if let Some(ids) = self.outgoing.get(&node_id) {
            for id in ids {
                if let Some(edge) = self.edges.get(id) {
                    f(edge);
                }
            }
        }
    }

    /// Returns the number of outgoing edges from `node_id` without materializing them.
    #[must_use]
    #[inline]
    pub fn outgoing_degree(&self, node_id: u64) -> usize {
        self.outgoing.get(&node_id).map_or(0, Vec::len)
    }

    /// Returns the number of incoming edges to `node_id` without materializing them.
    #[must_use]
    #[inline]
    pub fn incoming_degree(&self, node_id: u64) -> usize {
        self.incoming.get(&node_id).map_or(0, Vec::len)
    }

    /// Gets all incoming edges to a node.
    #[must_use]
    pub fn get_incoming(&self, node_id: u64) -> Vec<&GraphEdge> {
        self.resolve_edge_ids(self.incoming.get(&node_id))
    }

    /// Gets outgoing edges filtered by label using composite index - O(k) where k = result count.
    ///
    /// Uses the `outgoing_by_label` composite index for fast lookup instead of
    /// iterating through all outgoing edges (EPIC-019 US-003).
    #[must_use]
    pub fn get_outgoing_by_label(&self, node_id: u64, label: &str) -> Vec<&GraphEdge> {
        self.resolve_edge_ids(self.outgoing_by_label.get(&(node_id, label.to_string())))
    }

    /// Gets all edges with a specific label - O(k) where k = result count.
    ///
    /// Uses the `by_label` secondary index for fast lookup (EPIC-019 US-003).
    #[must_use]
    pub fn get_edges_by_label(&self, label: &str) -> Vec<&GraphEdge> {
        self.resolve_edge_ids(self.by_label.get(label))
    }

    /// Resolves edge IDs from an index entry into edge references.
    ///
    /// Shared lookup pattern used by `get_outgoing`, `get_incoming`,
    /// `get_outgoing_by_label`, and `get_edges_by_label`.
    #[inline]
    fn resolve_edge_ids(&self, ids: Option<&Vec<u64>>) -> Vec<&GraphEdge> {
        ids.map(|ids| ids.iter().filter_map(|id| self.edges.get(id)).collect())
            .unwrap_or_default()
    }

    /// Gets incoming edges filtered by label.
    #[must_use]
    pub fn get_incoming_by_label(&self, node_id: u64, label: &str) -> Vec<&GraphEdge> {
        self.get_incoming(node_id)
            .into_iter()
            .filter(|e| e.label() == label)
            .collect()
    }

    /// Checks if an edge with the given ID exists.
    #[must_use]
    pub fn contains_edge(&self, edge_id: u64) -> bool {
        self.edges.contains_key(&edge_id)
    }

    /// Returns the number of edges in the store.
    #[must_use]
    pub fn len(&self) -> usize {
        self.edges.len()
    }

    /// Returns true if the store contains no edges.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.edges.is_empty()
    }

    /// Returns all edges in the store.
    #[must_use]
    pub fn all_edges(&self) -> Vec<&GraphEdge> {
        self.edges.values().collect()
    }

    /// Removes an edge by ID.
    ///
    /// Cleans up all indices: outgoing, incoming, by_label, and outgoing_by_label.
    pub fn remove_edge(&mut self, edge_id: u64) {
        if let Some(edge) = self.edges.remove(&edge_id) {
            let source = edge.source();
            self.purge_outgoing_index(edge_id, source);
            self.purge_incoming_index(edge_id, edge.target());
            self.purge_label_indices(edge_id, source, edge.label());
            // Invalidate CSR snapshot (G1).
            self.csr_snapshot = None;
        }
    }

    /// Removes an edge by ID, only cleaning the outgoing index.
    ///
    /// Used by `ConcurrentEdgeStore` for cross-shard cleanup.
    /// Also cleans up label indices since they are maintained by source shard.
    pub fn remove_edge_outgoing_only(&mut self, edge_id: u64) {
        if let Some(edge) = self.edges.remove(&edge_id) {
            let source = edge.source();
            self.purge_outgoing_index(edge_id, source);
            self.purge_label_indices(edge_id, source, edge.label());
            // Invalidate CSR snapshot (G1).
            self.csr_snapshot = None;
        }
    }

    /// Removes an edge by ID, only cleaning the incoming index.
    ///
    /// Used by `ConcurrentEdgeStore` for cross-shard cleanup.
    pub fn remove_edge_incoming_only(&mut self, edge_id: u64) {
        if let Some(edge) = self.edges.remove(&edge_id) {
            self.purge_incoming_index(edge_id, edge.target());
            // Invalidate CSR snapshot (G1).
            self.csr_snapshot = None;
        }
    }

    /// Removes all edges connected to a node (cascade delete).
    ///
    /// Removes both outgoing and incoming edges, cleaning up all indices
    /// including label indices (EPIC-019 US-003).
    pub fn remove_node_edges(&mut self, node_id: u64) {
        // Collect edge IDs to remove (outgoing)
        let outgoing_ids: Vec<u64> = self.outgoing.remove(&node_id).unwrap_or_default();

        // Collect edge IDs to remove (incoming)
        let incoming_ids: Vec<u64> = self.incoming.remove(&node_id).unwrap_or_default();

        // Remove outgoing edges: clean incoming + label indices for each
        for edge_id in outgoing_ids {
            if let Some(edge) = self.edges.remove(&edge_id) {
                self.purge_incoming_index(edge_id, edge.target());
                self.purge_label_indices(edge_id, node_id, edge.label());
            }
        }

        // Remove incoming edges: clean outgoing + label indices for each
        for edge_id in incoming_ids {
            if let Some(edge) = self.edges.remove(&edge_id) {
                let source = edge.source();
                self.purge_outgoing_index(edge_id, source);
                self.purge_label_indices(edge_id, source, edge.label());
            }
        }

        // Invalidate CSR snapshot (G1).
        self.csr_snapshot = None;
    }

    /// Removes `edge_id` from the incoming index of `target_node`.
    #[inline]
    fn purge_incoming_index(&mut self, edge_id: u64, target_node: u64) {
        if let Some(ids) = self.incoming.get_mut(&target_node) {
            ids.retain(|&id| id != edge_id);
        }
    }

    /// Removes `edge_id` from the outgoing index of `source_node`.
    #[inline]
    fn purge_outgoing_index(&mut self, edge_id: u64, source_node: u64) {
        if let Some(ids) = self.outgoing.get_mut(&source_node) {
            ids.retain(|&id| id != edge_id);
        }
    }

    /// Removes `edge_id` from the `by_label` and `outgoing_by_label` indices (US-003).
    #[inline]
    fn purge_label_indices(&mut self, edge_id: u64, source_node: u64, label: &str) {
        if let Some(ids) = self.by_label.get_mut(label) {
            ids.retain(|&id| id != edge_id);
        }
        if let Some(ids) = self
            .outgoing_by_label
            .get_mut(&(source_node, label.to_string()))
        {
            ids.retain(|&id| id != edge_id);
        }
    }
}

// ---------------------------------------------------------------------------
// CSR snapshot methods (G1: zero-copy BFS)
// ---------------------------------------------------------------------------
impl EdgeStore {
    /// Builds a CSR snapshot from the current outgoing index.
    ///
    /// This pre-computes contiguous arrays of target IDs, edge IDs, and
    /// interned labels for all source nodes. After calling this, BFS
    /// traversal uses `with_neighbors()` for zero-copy `&[u64]` access
    /// instead of cloning full `GraphEdge` objects.
    ///
    /// # When to call
    ///
    /// - After loading from disk (graph is ready for reads)
    /// - After a batch of mutations, before a read-heavy phase
    ///
    /// The snapshot is automatically invalidated by any write operation.
    pub fn build_read_snapshot(&mut self) {
        let total_edges: usize = self.outgoing.values().map(Vec::len).sum();

        let mut index =
            FxHashMap::with_capacity_and_hasher(self.outgoing.len(), rustc_hash::FxBuildHasher);
        let mut targets = Vec::with_capacity(total_edges);
        let mut edge_ids_buf = Vec::with_capacity(total_edges);
        let mut label_indices = Vec::with_capacity(total_edges);
        let mut label_table: Vec<String> = Vec::new();
        let mut label_to_idx: FxHashMap<String, u32> =
            FxHashMap::with_capacity_and_hasher(16, rustc_hash::FxBuildHasher);

        for (&source_id, out_edge_ids) in &self.outgoing {
            let offset = targets.len();
            for &eid in out_edge_ids {
                if let Some(edge) = self.edges.get(&eid) {
                    targets.push(edge.target());
                    edge_ids_buf.push(eid);
                    let label = edge.label();
                    let lidx = *label_to_idx.entry(label.to_string()).or_insert_with(|| {
                        let idx = label_table.len();
                        label_table.push(label.to_string());
                        // Reason: label count bounded by schema size, never exceeds u32::MAX
                        #[allow(clippy::cast_possible_truncation)]
                        let idx_u32 = idx as u32;
                        idx_u32
                    });
                    label_indices.push(lidx);
                }
            }
            let len = targets.len() - offset;
            if len > 0 {
                index.insert(source_id, (offset, len));
            }
        }

        self.csr_snapshot = Some(CsrSnapshot {
            index,
            targets,
            edge_ids: edge_ids_buf,
            label_indices,
            label_table,
            label_to_idx,
        });
    }

    /// Returns a reference to the CSR snapshot, if built.
    #[must_use]
    #[inline]
    pub fn csr_snapshot(&self) -> Option<&CsrSnapshot> {
        self.csr_snapshot.as_ref()
    }

    /// Returns `true` if a CSR snapshot is available for zero-copy reads.
    #[must_use]
    #[inline]
    pub fn has_csr_snapshot(&self) -> bool {
        self.csr_snapshot.is_some()
    }

    /// Provides zero-copy access to neighbor target IDs via a callback.
    ///
    /// When a CSR snapshot is available, the callback receives a `&[u64]`
    /// slice directly from the contiguous snapshot buffer (zero allocation).
    /// When no snapshot exists, falls back to building a temporary `Vec<u64>`
    /// from the outgoing edge index.
    ///
    /// # Returns
    ///
    /// The value returned by `f`.
    #[inline]
    pub fn with_neighbors<F, R>(&self, source_id: u64, f: F) -> R
    where
        F: FnOnce(&[u64]) -> R,
    {
        if let Some(snapshot) = &self.csr_snapshot {
            f(snapshot.neighbors(source_id))
        } else {
            let ids: Vec<u64> = self
                .get_outgoing(source_id)
                .iter()
                .map(|e| e.target())
                .collect();
            f(&ids)
        }
    }

    /// Provides zero-copy access to `(target_id, edge_id)` pairs via callback.
    ///
    /// When a CSR snapshot exists, both slices come from contiguous memory.
    /// The callback receives `(targets: &[u64], edge_ids: &[u64])` where
    /// `targets[i]` is the neighbor reached by `edge_ids[i]`.
    #[inline]
    pub fn with_neighbor_edges<F, R>(&self, source_id: u64, f: F) -> R
    where
        F: FnOnce(&[u64], &[u64]) -> R,
    {
        if let Some(snapshot) = &self.csr_snapshot {
            f(snapshot.neighbors(source_id), snapshot.edge_ids(source_id))
        } else {
            let edges = self.get_outgoing(source_id);
            let targets: Vec<u64> = edges.iter().map(|e| e.target()).collect();
            let eids: Vec<u64> = edges.iter().map(|e| e.id()).collect();
            f(&targets, &eids)
        }
    }
}

impl PostcardPersistence for EdgeStore {}

// Inherent persistence methods that delegate to `PostcardPersistence`.
// Required so callers (e.g., `lifecycle.rs`) can use `EdgeStore::load_from_file`
// without importing the trait.
impl EdgeStore {
    /// Serializes the edge store to bytes using `postcard`.
    ///
    /// # Errors
    /// Returns an error if serialization fails.
    pub fn to_bytes(&self) -> std::result::Result<Vec<u8>, postcard::Error> {
        <Self as PostcardPersistence>::to_bytes(self)
    }

    /// Deserializes an edge store from bytes.
    ///
    /// # Errors
    /// Returns an error if deserialization fails (e.g., corrupted data).
    pub fn from_bytes(bytes: &[u8]) -> std::result::Result<Self, postcard::Error> {
        <Self as PostcardPersistence>::from_bytes(bytes)
    }

    /// Saves the edge store to a file.
    ///
    /// # Errors
    /// Returns an error if serialization or file I/O fails.
    pub fn save_to_file(&self, path: &std::path::Path) -> std::io::Result<()> {
        <Self as PostcardPersistence>::save_to_file(self, path)
    }

    /// Loads an edge store from a file.
    ///
    /// Automatically builds a CSR snapshot after loading for zero-copy
    /// BFS traversal (G1).
    ///
    /// # Errors
    /// Returns an error if file I/O or deserialization fails.
    pub fn load_from_file(path: &std::path::Path) -> std::io::Result<Self> {
        let mut store = <Self as PostcardPersistence>::load_from_file(path)?;
        store.build_read_snapshot();
        Ok(store)
    }
}
