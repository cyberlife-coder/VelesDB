//! HNSW Layer implementation.
//!
//! A single layer in the HNSW hierarchy containing node adjacency lists.

use parking_lot::RwLock;
use std::sync::atomic::{AtomicU32, Ordering};

/// Unique identifier for a node in the graph.
pub type NodeId = usize;

/// No anchor: the entry point, a node not anchored yet, or one a loaded graph
/// could not reach.
const NO_ANCHOR: u32 = u32::MAX;

/// A single layer in the HNSW hierarchy.
#[derive(Debug)]
pub struct Layer {
    /// Adjacency list: node_id -> list of neighbor node_ids
    pub(crate) neighbors: Vec<RwLock<Vec<NodeId>>>,
    /// Base layer only: each node's anchor, the node whose list holds the one
    /// edge to it that eviction never removes (#2259). Four bytes a slot, the
    /// width node ids already have on disk. Empty in upper layers.
    anchors: Vec<AtomicU32>,
    /// Whether this is the base layer, the one that records anchors.
    base: bool,
}

impl Layer {
    /// Creates a new upper layer with the given capacity.
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            neighbors: (0..capacity).map(|_| RwLock::new(Vec::new())).collect(),
            anchors: Vec::new(),
            base: false,
        }
    }

    /// Creates the base layer (layer 0), which also records every node's
    /// anchor.
    pub(crate) fn new_base(capacity: usize) -> Self {
        Self {
            neighbors: (0..capacity).map(|_| RwLock::new(Vec::new())).collect(),
            anchors: (0..capacity).map(|_| AtomicU32::new(NO_ANCHOR)).collect(),
            base: true,
        }
    }

    /// Ensures the layer has capacity for the given node_id.
    pub(crate) fn ensure_capacity(&mut self, node_id: NodeId) {
        while self.neighbors.len() <= node_id {
            self.neighbors.push(RwLock::new(Vec::new()));
        }
        if self.base {
            while self.anchors.len() <= node_id {
                self.anchors.push(AtomicU32::new(NO_ANCHOR));
            }
        }
    }

    /// The layer at `level` of the hierarchy: level 0, the base layer, records
    /// anchors, whichever path creates it.
    pub(crate) fn at_level(level: usize, capacity: usize) -> Self {
        if level == 0 {
            Self::new_base(capacity)
        } else {
            Self::new(capacity)
        }
    }

    /// Whether this layer records anchors: the base layer only.
    pub(in crate::index::hnsw::native) fn records_anchors(&self) -> bool {
        self.base
    }

    /// The node whose list holds `node`'s protected edge, if `node` has one.
    #[inline]
    pub(in crate::index::hnsw::native) fn anchor_of(&self, node: NodeId) -> Option<NodeId> {
        self.anchors
            .get(node)
            .map(|anchor| anchor.load(Ordering::Acquire))
            .filter(|&anchor| anchor != NO_ANCHOR)
            .and_then(|anchor| NodeId::try_from(anchor).ok())
    }

    /// Records `parent` as `node`'s anchor.
    ///
    /// The caller holds `parent`'s list lock and has just made sure `node` is
    /// in that list: an evictor must take the same lock, so it can never see
    /// the anchor without the edge, nor remove the edge after the anchor.
    ///
    /// Node ids are persisted as `u32`, so every one fits; an id that did not
    /// would only leave `node` unprotected.
    #[inline]
    pub(in crate::index::hnsw::native) fn set_anchor(&self, node: NodeId, parent: NodeId) {
        if let (Some(anchor), Ok(parent)) = (self.anchors.get(node), u32::try_from(parent)) {
            anchor.store(parent, Ordering::Release);
        }
    }

    /// Drops `node`'s anchor. The entry point is the tree's root and has none;
    /// clearing only unprotects an edge, so no list lock is needed.
    #[inline]
    pub(in crate::index::hnsw::native) fn clear_anchor(&self, node: NodeId) {
        if let Some(anchor) = self.anchors.get(node) {
            anchor.store(NO_ANCHOR, Ordering::Release);
        }
    }

    /// Gets the neighbors of a node.
    #[allow(dead_code)] // Reason: Used in tests (layer_tests, graph_tests) for adjacency verification
    #[inline]
    pub(crate) fn get_neighbors(&self, node_id: NodeId) -> Vec<NodeId> {
        if node_id < self.neighbors.len() {
            self.neighbors[node_id].read().clone()
        } else {
            Vec::new()
        }
    }

    /// Runs a closure with immutable access to a node's adjacency list under a read lock.
    #[inline]
    pub(crate) fn with_neighbors<R>(
        &self,
        node_id: NodeId,
        f: impl FnOnce(&[NodeId]) -> R,
    ) -> Option<R> {
        if node_id < self.neighbors.len() {
            let guard = self.neighbors[node_id].read();
            Some(f(&guard))
        } else {
            None
        }
    }

    /// Sets the neighbors for a node.
    #[inline]
    pub(crate) fn set_neighbors(&self, node_id: NodeId, neighbors: Vec<NodeId>) {
        if node_id < self.neighbors.len() {
            *self.neighbors[node_id].write() = neighbors;
        }
    }

    /// Mutates the neighbors for a node in-place under a single write lock.
    #[inline]
    pub(crate) fn with_neighbors_mut<R>(
        &self,
        node_id: NodeId,
        f: impl FnOnce(&mut Vec<NodeId>) -> R,
    ) -> Option<R> {
        if node_id < self.neighbors.len() {
            let mut guard = self.neighbors[node_id].write();
            Some(f(&mut guard))
        } else {
            None
        }
    }

    /// Adds a neighbor to a node's adjacency list.
    #[allow(dead_code)] // Reason: Used in tests for graph construction verification
    pub(super) fn add_neighbor(&self, node_id: NodeId, neighbor: NodeId) {
        if node_id < self.neighbors.len() {
            self.neighbors[node_id].write().push(neighbor);
        }
    }

    /// Remaps all neighbor IDs using the provided old-to-new mapping.
    ///
    /// After graph reordering, node IDs change. This method updates every
    /// neighbor reference in the layer to use the new IDs, and reorders the
    /// adjacency lists themselves so that slot `new_id` contains the neighbors
    /// of the node that was formerly at `old_id`.
    pub(crate) fn remap_ids(&mut self, old_to_new: &[usize]) {
        let count = old_to_new.len();

        // Phase 1: Remap neighbor IDs within each adjacency list.
        for lock in &self.neighbors {
            let mut neighbors = lock.write();
            for id in neighbors.iter_mut() {
                if *id < count {
                    *id = old_to_new[*id];
                }
            }
        }

        // Phase 2: Reorder the adjacency lists themselves.
        // Extract all lists (releases write locks), then apply the old→new
        // permutation in-place via cycle decomposition so that slot new_id
        // ends up with the list that was at old_id — without allocating a
        // second full-size Vec<Vec<NodeId>>.
        let mut extracted: Vec<Vec<NodeId>> = self
            .neighbors
            .iter()
            .map(|lock| std::mem::take(&mut *lock.write()))
            .collect();

        // Build inverse mapping: new_to_old[new_id] = old_id, so the
        // cycle-decomposition below can be driven forward ("output slot i
        // receives element from slot new_to_old[i]").
        let n = count.min(extracted.len());
        let mut new_to_old: Vec<usize> = (0..n).collect();
        for (old, &new) in old_to_new.iter().enumerate() {
            if old < extracted.len() && new < n {
                new_to_old[new] = old;
            }
        }

        // Apply permutation to the first `n` slots in-place.
        for i in 0..n {
            let mut j = i;
            while new_to_old[j] != i {
                let k = new_to_old[j];
                extracted.swap(j, k);
                new_to_old[j] = j;
                j = k;
            }
            new_to_old[j] = j;
        }

        // Write back all slots (slots >= n were already cleared by mem::take).
        for (i, lock) in self.neighbors.iter().enumerate() {
            *lock.write() = std::mem::take(&mut extracted[i]);
        }

        self.remap_anchors(old_to_new);
    }

    /// Moves every anchor to its node's new id and renames the parent it
    /// points at, exactly as `remap_ids` does for the adjacency lists.
    fn remap_anchors(&mut self, old_to_new: &[usize]) {
        let rename = |id: NodeId| old_to_new.get(id).copied().unwrap_or(id);
        let old: Vec<u32> = self
            .anchors
            .iter_mut()
            .map(|anchor| std::mem::replace(anchor.get_mut(), NO_ANCHOR))
            .collect();
        for (node, parent) in old.into_iter().enumerate() {
            if parent == NO_ANCHOR {
                continue;
            }
            let Ok(parent) = NodeId::try_from(parent) else {
                continue;
            };
            if let (Some(slot), Ok(renamed)) = (
                self.anchors.get_mut(rename(node)),
                u32::try_from(rename(parent)),
            ) {
                *slot.get_mut() = renamed;
            }
        }
    }
}
