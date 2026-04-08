//! Zero-copy CSR (Compressed Sparse Row) snapshot for cache-friendly BFS traversal.
//!
//! Extracted from `edge.rs` to reduce NLOC. Contains:
//! - `CsrSnapshot`: Immutable CSR snapshot of the graph
//! - `SnapshotBuilder`: Builds a `CsrSnapshot` from an `EdgeStore`
//! - `EdgePredicate` trait + `LabelFilter` / `NoFilter` implementations
//! - `AdjacencySource` trait for generic traversal

use super::edge::EdgeStore;
use super::label_table::{LabelId, LabelTable};
use rustc_hash::{FxHashMap, FxHashSet};

// ---------------------------------------------------------------------------
// EdgePredicate trait and filters (Task 7: predicate pushdown)
// ---------------------------------------------------------------------------

/// Trait for predicate pushdown filtering in [`CsrSnapshot`].
///
/// Implementations evaluate whether an edge should be included in traversal
/// results directly at the CSR level, avoiding materialisation of non-matching
/// edges.
pub trait EdgePredicate: Send + Sync {
    /// Returns `true` if the edge `(target, edge_id, label_id)` should be
    /// included in the result set.
    fn matches(&self, target: u64, edge_id: u64, label_id: LabelId) -> bool;
}

/// Filters edges by a set of allowed [`LabelId`]s.
///
/// Only edges whose label is in the `allowed` set pass the predicate.
pub struct LabelFilter {
    allowed: FxHashSet<LabelId>,
}

impl LabelFilter {
    /// Creates a new `LabelFilter` accepting only the given label IDs.
    #[must_use]
    pub fn new(allowed: FxHashSet<LabelId>) -> Self {
        Self { allowed }
    }
}

impl EdgePredicate for LabelFilter {
    #[inline]
    fn matches(&self, _target: u64, _edge_id: u64, label_id: LabelId) -> bool {
        self.allowed.contains(&label_id)
    }
}

/// Accepts all edges (no-op predicate).
///
/// Optimised away by monomorphisation — the compiler inlines the constant
/// `true` return, producing zero overhead compared to an unfiltered path.
pub struct NoFilter;

impl EdgePredicate for NoFilter {
    #[inline]
    fn matches(&self, _target: u64, _edge_id: u64, _label_id: LabelId) -> bool {
        true
    }
}

// ---------------------------------------------------------------------------
// AdjacencySource trait (Task 9: generic BFS)
// ---------------------------------------------------------------------------

/// Source of adjacency data for traversal algorithms.
///
/// Abstracts neighbor access so that BFS/DFS algorithms can work with
/// either [`CsrSnapshot`] (zero-copy) or [`EdgeStore`] (legacy) without
/// code duplication.
pub trait AdjacencySource {
    /// Returns the target node IDs reachable from `node_id`.
    fn neighbors(&self, node_id: u64) -> Vec<u64>;
}

impl AdjacencySource for CsrSnapshot {
    /// Returns neighbors from the CSR contiguous array (copies to Vec).
    #[inline]
    fn neighbors(&self, node_id: u64) -> Vec<u64> {
        self.neighbors(node_id).to_vec()
    }
}

impl AdjacencySource for EdgeStore {
    /// Returns outgoing neighbor target IDs from the edge index.
    #[inline]
    fn neighbors(&self, node_id: u64) -> Vec<u64> {
        self.get_outgoing(node_id)
            .iter()
            .map(|e| e.target())
            .collect()
    }
}

// ---------------------------------------------------------------------------
// CsrSnapshot
// ---------------------------------------------------------------------------

/// Immutable CSR (Compressed Sparse Row) snapshot of the graph for zero-copy traversals.
///
/// All arrays are contiguous in memory for optimal cache locality during BFS/DFS.
///
/// # Memory layout
///
/// ```text
/// offsets[i]..offsets[i+1] = range of neighbors for node at index i
/// targets[offset]          = target node_id
/// edge_ids[offset]         = edge ID
/// label_ids[offset]        = interned LabelId
/// ```
///
/// `offsets` has length `node_count + 1`, where `offsets[node_count] == targets.len()`.
#[derive(Debug, Clone)]
pub struct CsrSnapshot {
    /// Offset array: `offsets[i]..offsets[i+1]` = neighbor range for node at index `i`.
    /// Length = `node_count + 1`. `offsets[node_count] == targets.len()`.
    offsets: Vec<usize>,
    /// Contiguous storage of target node IDs for all outgoing edges.
    targets: Vec<u64>,
    /// Contiguous storage of edge IDs, parallel to `targets`.
    edge_ids: Vec<u64>,
    /// Contiguous storage of interned label IDs, parallel to `targets`.
    label_ids: Vec<LabelId>,
    /// Mapping `node_id → index` in the offsets array for O(1) lookup.
    node_to_index: FxHashMap<u64, usize>,
    /// Mapping `index → node_id` (inverse of `node_to_index`).
    index_to_node: Vec<u64>,
    /// Interned label strings for label-based filtering.
    label_table: Vec<String>,
    /// Reverse map: label string → label index for O(1) lookup.
    label_to_idx: FxHashMap<String, u32>,
}

impl CsrSnapshot {
    /// Returns the `(offset, len)` range for a node, or `None` if absent.
    #[inline]
    fn range_of(&self, node_id: u64) -> Option<(usize, usize)> {
        let &idx = self.node_to_index.get(&node_id)?;
        let start = self.offsets[idx];
        let end = self.offsets[idx + 1];
        Some((start, end))
    }

    /// Returns neighbor target IDs for a source node as a zero-copy slice.
    #[must_use]
    #[inline]
    pub fn neighbors(&self, node_id: u64) -> &[u64] {
        if let Some((start, end)) = self.range_of(node_id) {
            &self.targets[start..end]
        } else {
            &[]
        }
    }

    /// Returns edge IDs for a source node as a zero-copy slice.
    ///
    /// Parallel to `neighbors()`: `edge_ids[i]` is the edge connecting
    /// `node_id` to `neighbors()[i]`.
    #[must_use]
    #[inline]
    pub fn edge_ids(&self, node_id: u64) -> &[u64] {
        if let Some((start, end)) = self.range_of(node_id) {
            &self.edge_ids[start..end]
        } else {
            &[]
        }
    }

    /// Returns interned label IDs for a source node as a zero-copy slice.
    ///
    /// Parallel to `neighbors()`: `label_ids[i]` is the label of the edge
    /// connecting `node_id` to `neighbors()[i]`.
    #[must_use]
    #[inline]
    pub fn label_ids(&self, node_id: u64) -> &[LabelId] {
        if let Some((start, end)) = self.range_of(node_id) {
            &self.label_ids[start..end]
        } else {
            &[]
        }
    }

    /// Returns the label string for a neighbor at position `neighbor_idx`
    /// relative to the node's offset.
    ///
    /// Returns `None` if `source_id` is absent or `neighbor_idx` is out of range.
    #[must_use]
    #[inline]
    pub fn label_at(&self, source_id: u64, neighbor_idx: usize) -> Option<&str> {
        let (start, end) = self.range_of(source_id)?;
        if neighbor_idx >= end - start {
            return None;
        }
        let label_id = self.label_ids[start + neighbor_idx];
        self.label_table
            .get(label_id.as_u32() as usize)
            .map(String::as_str)
    }

    /// Returns the outgoing degree of a node.
    #[must_use]
    #[inline]
    pub fn degree(&self, node_id: u64) -> usize {
        if let Some((start, end)) = self.range_of(node_id) {
            end - start
        } else {
            0
        }
    }

    /// Returns `true` if the node exists in this snapshot.
    #[must_use]
    #[inline]
    pub fn contains_node(&self, node_id: u64) -> bool {
        self.node_to_index.contains_key(&node_id)
    }

    /// Returns the number of source nodes in this snapshot.
    #[must_use]
    #[inline]
    pub fn node_count(&self) -> usize {
        self.index_to_node.len()
    }

    /// Returns the total number of outgoing edges in this snapshot.
    #[must_use]
    #[inline]
    pub fn edge_count(&self) -> usize {
        self.targets.len()
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

    /// Returns an iterator over neighbors that match the given predicate.
    ///
    /// Only edges for which `predicate.matches(target, edge_id, label_id)`
    /// returns `true` are yielded. Non-matching edges are skipped without
    /// materialisation.
    ///
    /// Each yielded item is `(target_id, edge_id, label_id)`.
    pub fn neighbors_filtered<'a, P: EdgePredicate>(
        &'a self,
        node_id: u64,
        predicate: &'a P,
    ) -> impl Iterator<Item = (u64, u64, LabelId)> + 'a {
        let (start, end) = self.range_of(node_id).unwrap_or((0, 0));
        (start..end).filter_map(move |i| {
            let target = self.targets[i];
            let eid = self.edge_ids[i];
            let lid = self.label_ids[i];
            if predicate.matches(target, eid, lid) {
                Some((target, eid, lid))
            } else {
                None
            }
        })
    }

    /// Returns a reference to the internal offsets array (for testing/validation).
    #[cfg(test)]
    pub(crate) fn offsets(&self) -> &[usize] {
        &self.offsets
    }
}

// ---------------------------------------------------------------------------
// SnapshotBuilder
// ---------------------------------------------------------------------------

/// Builds a [`CsrSnapshot`] from an [`EdgeStore`] and [`LabelTable`].
///
/// This is a stateless namespace — no persistent state is held.
/// Construction complexity is O(N + E) where N = nodes, E = edges.
pub(crate) struct SnapshotBuilder;

impl SnapshotBuilder {
    /// Builds a `CsrSnapshot` from the given `EdgeStore` and `LabelTable`.
    ///
    /// # Algorithm
    ///
    /// 1. Collect all unique source `node_id`s from `edge_store.outgoing`.
    /// 2. Sort for deterministic layout.
    /// 3. Build `node_to_index` / `index_to_node`.
    /// 4. For each node in order, iterate outgoing edges and fill
    ///    `targets`, `edge_ids`, `label_ids`.
    /// 5. Accumulate `offsets`.
    pub fn build(edge_store: &EdgeStore, _label_table: &LabelTable) -> CsrSnapshot {
        // 1. Collect unique source node_ids
        let mut node_ids: Vec<u64> = edge_store.outgoing_keys();

        // 2. Sort for deterministic layout
        node_ids.sort_unstable();

        let node_count = node_ids.len();
        let total_edges: usize = edge_store.total_outgoing_edges();

        // 3. Build node_to_index and index_to_node
        let mut node_to_index =
            FxHashMap::with_capacity_and_hasher(node_count, rustc_hash::FxBuildHasher);
        for (idx, &nid) in node_ids.iter().enumerate() {
            node_to_index.insert(nid, idx);
        }

        // 4 & 5. Fill arrays
        let mut offsets = Vec::with_capacity(node_count + 1);
        let mut targets = Vec::with_capacity(total_edges);
        let mut edge_ids_buf = Vec::with_capacity(total_edges);
        let mut label_ids_buf: Vec<LabelId> = Vec::with_capacity(total_edges);
        let mut label_table_vec: Vec<String> = Vec::new();
        let mut label_to_idx: FxHashMap<String, u32> =
            FxHashMap::with_capacity_and_hasher(16, rustc_hash::FxBuildHasher);

        for &nid in &node_ids {
            offsets.push(targets.len());
            edge_store.for_each_outgoing_edge(nid, |edge| {
                targets.push(edge.target());
                edge_ids_buf.push(edge.id());

                // Always use local interning for label_ids stored in CSR.
                // label_at() resolves against the local label_table vec.
                let label_str = edge.label();
                let local_idx = *label_to_idx
                    .entry(label_str.to_string())
                    .or_insert_with(|| {
                        let idx = label_table_vec.len();
                        label_table_vec.push(label_str.to_string());
                        #[allow(clippy::cast_possible_truncation)]
                        // SAFETY: label count bounded by schema size
                        {
                            idx as u32
                        }
                    });
                label_ids_buf.push(LabelId::from_u32(local_idx));
            });
        }
        // Final offset sentinel
        offsets.push(targets.len());

        CsrSnapshot {
            offsets,
            targets,
            edge_ids: edge_ids_buf,
            label_ids: label_ids_buf,
            node_to_index,
            index_to_node: node_ids,
            label_table: label_table_vec,
            label_to_idx,
        }
    }

    /// Creates an empty `CsrSnapshot` (no nodes, no edges).
    #[must_use]
    #[allow(dead_code)] // Used by tests and ConcurrentEdgeStore (Task 5)
    pub fn empty() -> CsrSnapshot {
        CsrSnapshot {
            offsets: vec![0],
            targets: Vec::new(),
            edge_ids: Vec::new(),
            label_ids: Vec::new(),
            node_to_index: FxHashMap::default(),
            index_to_node: Vec::new(),
            label_table: Vec::new(),
            label_to_idx: FxHashMap::default(),
        }
    }
}
