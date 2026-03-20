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
use super::NativeHnsw;
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

        let Some(entry) = *self.entry_point.read() else {
            return Ok(());
        };

        let permutation = self.compute_bfs_order(entry, count);
        if permutation.is_empty() {
            return Ok(());
        }

        self.apply_permutation(&permutation)
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
    fn append_unvisited(&self, visited: &[bool], order: &mut Vec<NodeId>) {
        for (node, &was_visited) in visited.iter().enumerate() {
            if !was_visited {
                order.push(node);
            }
        }
    }

    /// Applies a permutation to vectors, neighbor lists, and the entry point.
    fn apply_permutation(&self, new_order: &[NodeId]) -> crate::error::Result<()> {
        let count = new_order.len();

        let old_to_new = Self::build_reverse_mapping(new_order, count);

        self.reorder_vectors(new_order)?;
        self.remap_neighbor_ids(&old_to_new);
        self.update_entry_point(&old_to_new, count);

        Ok(())
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
    fn update_entry_point(&self, old_to_new: &[usize], count: usize) {
        let mut ep = self.entry_point.write();
        if let Some(old_ep) = *ep {
            if old_ep < count {
                *ep = Some(old_to_new[old_ep]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::distance::CpuDistance;
    use super::*;
    use crate::distance::DistanceMetric;

    /// Creates a small test index with `n` vectors of dimension `dim`.
    fn build_test_index(n: usize, dim: usize) -> NativeHnsw<CpuDistance> {
        let distance = CpuDistance::new(DistanceMetric::Euclidean);
        let hnsw = NativeHnsw::new(distance, 8, 32, n);
        for i in 0..n {
            // Reason: cast_precision_loss acceptable for test data generation.
            #[allow(clippy::cast_precision_loss)]
            let v: Vec<f32> = (0..dim).map(|d| (i * dim + d) as f32).collect();
            hnsw.insert(v).unwrap();
        }
        hnsw
    }

    #[test]
    fn reorder_skips_small_index() {
        let hnsw = build_test_index(50, 4);
        // Should be a no-op for <1000 vectors
        assert!(hnsw.reorder_for_locality().is_ok());
    }

    #[test]
    fn reorder_preserves_search_results() {
        let hnsw = build_test_index(1200, 4);

        // Search before reorder
        let query = vec![1.0, 2.0, 3.0, 4.0];
        let before = hnsw.search(&query, 5, 64);
        let _before_ids: Vec<NodeId> = before.iter().map(|(id, _)| *id).collect();

        // Reorder
        hnsw.reorder_for_locality().unwrap();

        // Search after reorder — results should contain the same vectors
        // (IDs change but the distances should be identical)
        let after = hnsw.search(&query, 5, 64);
        assert_eq!(before.len(), after.len(), "Result count changed");

        // Distances should match (order may differ by tie-breaking)
        let mut before_dists: Vec<f32> = before.iter().map(|(_, d)| *d).collect();
        let mut after_dists: Vec<f32> = after.iter().map(|(_, d)| *d).collect();
        before_dists.sort_by(f32::total_cmp);
        after_dists.sort_by(f32::total_cmp);
        for (b, a) in before_dists.iter().zip(after_dists.iter()) {
            assert!(
                (b - a).abs() < 1e-5,
                "Distance mismatch: before={b}, after={a}"
            );
        }

        // Verify entry point is still valid
        let ep = *hnsw.entry_point.read();
        assert!(ep.is_some(), "Entry point lost after reorder");
        let ep_id = ep.unwrap();
        assert!(
            ep_id < hnsw.count.load(Ordering::Relaxed),
            "Entry point out of bounds"
        );

        // Verify all IDs in results are within bounds
        for (id, _) in &after {
            assert!(*id < hnsw.count.load(Ordering::Relaxed));
        }
    }

    #[test]
    fn bfs_order_covers_all_nodes() {
        let hnsw = build_test_index(1200, 4);
        let count = hnsw.count.load(Ordering::Relaxed);
        let ep = hnsw.entry_point.read().unwrap();
        let order = hnsw.compute_bfs_order(ep, count);

        assert_eq!(order.len(), count, "BFS order must cover all nodes");

        // Verify it's a valid permutation (each node appears exactly once)
        let mut seen = vec![false; count];
        for &id in &order {
            assert!(id < count, "BFS order contains out-of-bounds ID: {id}");
            assert!(!seen[id], "BFS order contains duplicate ID: {id}");
            seen[id] = true;
        }
    }

    #[test]
    fn reverse_mapping_is_inverse() {
        let new_order = vec![3, 1, 4, 0, 2];
        let old_to_new = NativeHnsw::<CpuDistance>::build_reverse_mapping(&new_order, 5);
        // new_order[0] = 3  => old_to_new[3] = 0
        // new_order[1] = 1  => old_to_new[1] = 1
        // new_order[2] = 4  => old_to_new[4] = 2
        // new_order[3] = 0  => old_to_new[0] = 3
        // new_order[4] = 2  => old_to_new[2] = 4
        assert_eq!(old_to_new, vec![3, 1, 4, 0, 2]);
    }
}
