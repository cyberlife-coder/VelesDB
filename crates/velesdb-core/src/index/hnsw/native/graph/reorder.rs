//! BFS-based graph reordering for improved cache locality during HNSW search.
//!
//! After index construction, vectors are stored in insertion order. Reordering
//! them in BFS traversal order from the entry point improves spatial locality
//! when following graph edges during search, reducing cache misses by 15-30%.
//!
//! Reference: "Graph Reordering for Cache-Efficient Near Neighbor Search"
//! (arXiv:2104.03221, NeurIPS 2022).

use super::super::distance::DistanceEngine;
use super::super::layer::NodeId;
use super::{NativeHnsw, NO_ENTRY_POINT};
use std::collections::VecDeque;
use std::sync::atomic::Ordering;

/// Minimum element count below which reordering provides no measurable benefit.
/// At <1000 vectors, the entire working set fits in L2 cache.
const REORDER_THRESHOLD: usize = 1000;

impl<D: DistanceEngine> NativeHnsw<D> {
    /// Reorders graph nodes in BFS traversal order for improved cache locality.
    ///
    /// After reordering, vectors that are close in the graph are also close
    /// in memory, reducing cache misses during search traversal.
    ///
    /// # When to call
    ///
    /// - After `build()` completes for a static index
    /// - After compaction of a dynamic index
    /// - Not needed for small indices (< 1000 vectors)
    ///
    /// # Errors
    ///
    /// Returns an error if vector storage reordering fails.
    pub fn reorder_for_locality(&self) -> crate::error::Result<()> {
        let count = self.count.load(Ordering::Relaxed);
        if count < REORDER_THRESHOLD {
            return Ok(());
        }

        let entry = self.entry_point.load(Ordering::Acquire);
        if entry == NO_ENTRY_POINT {
            return Ok(());
        }

        let permutation = self.compute_bfs_order(entry, count);
        if permutation.is_empty() {
            return Ok(());
        }

        self.apply_permutation(&permutation)?;

        // Reordering rewrites both the vector buffer AND every neighbour
        // list, so any cached CSR / flat-vector snapshot built from the
        // pre-reorder topology is now stale. The contract on
        // `invalidate_gpu_caches` is "call after every mutation that
        // changes the set of active nodes or their vector data" — an
        // in-place permutation qualifies. This was a pre-existing gap
        // (PR #626 never invalidated here either); folding the fix into
        // PR-B of #634 since the whole point of the unified version
        // counter is to close this class of bug.
        #[cfg(feature = "gpu")]
        self.invalidate_gpu_caches();

        Ok(())
    }

    /// Computes BFS traversal order starting from the entry point on layer 0.
    ///
    /// Returns a permutation where `result[i]` is the old node ID that should
    /// occupy position `i` after reordering. Disconnected nodes are appended
    /// in their original order after the BFS component.
    fn compute_bfs_order(&self, entry: NodeId, count: usize) -> Vec<NodeId> {
        let layers = self.layers.read();
        if layers.is_empty() {
            return Vec::new();
        }

        let mut order = Vec::with_capacity(count);
        let mut visited = vec![false; count];
        let mut queue = VecDeque::with_capacity(count);

        if entry < count {
            visited[entry] = true;
            queue.push_back(entry);
        }

        self.bfs_walk(&layers[0], &mut queue, &mut visited, &mut order, count);
        self.append_unvisited(&visited, &mut order);

        order
    }

    /// Runs BFS on the given layer, draining the queue and appending nodes to `order`.
    #[allow(clippy::unused_self)] // Reason: method receiver for future per-graph config
    fn bfs_walk(
        &self,
        layer: &super::super::layer::Layer,
        queue: &mut VecDeque<NodeId>,
        visited: &mut [bool],
        order: &mut Vec<NodeId>,
        count: usize,
    ) {
        while let Some(node) = queue.pop_front() {
            order.push(node);
            let _ = layer.with_neighbors(node, |neighbors| {
                for &neighbor in neighbors {
                    if neighbor < count && !visited[neighbor] {
                        visited[neighbor] = true;
                        queue.push_back(neighbor);
                    }
                }
            });
        }
    }

    /// Appends any unvisited nodes (disconnected components) to `order`.
    #[allow(clippy::unused_self)] // Reason: method receiver for future per-graph config
    fn append_unvisited(&self, visited: &[bool], order: &mut Vec<NodeId>) {
        for (node, &was_visited) in visited.iter().enumerate() {
            if !was_visited {
                order.push(node);
            }
        }
    }

    /// Applies a permutation to vectors, neighbor lists, and the entry point.
    ///
    /// After reordering, builds a PDX columnar layout from the reordered
    /// vectors for SIMD-parallel distance computation.
    fn apply_permutation(&self, new_order: &[NodeId]) -> crate::error::Result<()> {
        let count = new_order.len();

        let old_to_new = Self::build_reverse_mapping(new_order, count);

        self.reorder_vectors(new_order)?;
        self.remap_neighbor_ids(&old_to_new);
        self.update_entry_point(&old_to_new, count);
        self.build_columnar_layout();

        Ok(())
    }

    /// Builds a PDX block-columnar layout from the current vector storage.
    ///
    /// This transposes row-major vectors into 64-vector blocks where each
    /// dimension is contiguous, enabling SIMD-parallel distance computation.
    fn build_columnar_layout(&self) {
        let vectors_guard = self.vectors.read();
        if let Some(vectors) = vectors_guard.as_ref() {
            let pdx = super::super::columnar_vectors::ColumnarVectors::from_contiguous(vectors);
            *self.columnar.write() = Some(pdx);
        }
    }

    /// Builds a reverse mapping: `result[old_id] = new_id`.
    fn build_reverse_mapping(new_order: &[NodeId], count: usize) -> Vec<usize> {
        let mut old_to_new = vec![0usize; count];
        for (new_id, &old_id) in new_order.iter().enumerate() {
            if old_id < count {
                old_to_new[old_id] = new_id;
            }
        }
        old_to_new
    }

    /// Reorders vector storage according to the given permutation.
    fn reorder_vectors(&self, new_order: &[NodeId]) -> crate::error::Result<()> {
        let mut guard = self.vectors.write();
        if let Some(storage) = guard.as_mut() {
            storage.reorder(new_order)?;
        }
        Ok(())
    }

    /// Remaps all neighbor IDs in all layers according to the mapping.
    fn remap_neighbor_ids(&self, old_to_new: &[usize]) {
        let mut layers = self.layers.write();
        for layer in layers.iter_mut() {
            layer.remap_ids(old_to_new);
        }
    }

    /// Updates the entry point to its new ID after permutation.
    ///
    /// Called during `reorder_for_locality()` which runs single-threaded
    /// after all inserts complete. No concurrent promotions are possible,
    /// so a direct atomic store with `Release` ordering is sufficient.
    fn update_entry_point(&self, old_to_new: &[usize], count: usize) {
        let old_ep = self.entry_point.load(Ordering::Acquire);
        if old_ep != NO_ENTRY_POINT && old_ep < count {
            self.entry_point
                .store(old_to_new[old_ep], Ordering::Release);
        }
    }
}

#[cfg(test)]
#[path = "reorder_tests.rs"]
mod tests;
