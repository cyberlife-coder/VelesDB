//! Clustered index for cache-friendly low-degree vertex storage.
//!
//! This module provides a memory-efficient storage structure that groups
//! multiple neighbor sets into contiguous blocks for optimal cache utilization.
//!
//! # EPIC-020 US-004: Clustered Index for Low-Degree Vertices
//!
//! ## Design
//!
//! Instead of allocating separate `Vec<u64>` per node (48+ bytes overhead each),
//! we store all neighbor sets in a single contiguous buffer with an index
//! mapping node IDs to (offset, length) pairs.
//!
//! ## References
//!
//! - `RapidStore` Section 6.3: "low-degree vertices use small arrays further
//!   grouped into a tree to optimize memory usage"

// SAFETY: Numeric casts in clustered index are intentional:
// - usize->f64 for fragmentation ratio: precision loss acceptable (0-1 ratio)
// - Values bounded by buffer sizes, used for statistics only
#![allow(clippy::cast_precision_loss)]

use super::degree_router::EdgeIndex;
use rustc_hash::FxHashMap;

/// Fragmentation threshold (30%) that triggers automatic compaction.
const FRAGMENTATION_THRESHOLD: f64 = 0.30;

/// Clustered index storing multiple neighbor sets in contiguous memory.
///
/// Optimized for low-degree vertices (< 100 neighbors) where memory overhead
/// of individual allocations dominates.
#[derive(Debug, Clone)]
pub struct ClusteredIndex {
    /// Contiguous storage for all neighbor targets
    data: Vec<u64>,
    /// Maps `node_id` -> (offset, length) in data
    index: FxHashMap<u64, (usize, usize)>,
    /// Free slots available for reuse: (offset, length)
    free_slots: Vec<(usize, usize)>,
    /// Total bytes marked as free (for fragmentation calculation)
    free_bytes: usize,
}

impl ClusteredIndex {
    /// Creates a new empty clustered index.
    #[must_use]
    pub fn new() -> Self {
        Self {
            data: Vec::new(),
            index: FxHashMap::default(),
            free_slots: Vec::new(),
            free_bytes: 0,
        }
    }

    /// Creates a clustered index with pre-allocated capacity.
    #[must_use]
    pub fn with_capacity(node_capacity: usize, data_capacity: usize) -> Self {
        Self {
            data: Vec::with_capacity(data_capacity),
            index: FxHashMap::with_capacity_and_hasher(node_capacity, rustc_hash::FxBuildHasher),
            free_slots: Vec::new(),
            free_bytes: 0,
        }
    }

    /// Gets the neighbors for a node as a slice.
    #[must_use]
    pub fn get_neighbors(&self, node_id: u64) -> &[u64] {
        if let Some(&(offset, length)) = self.index.get(&node_id) {
            &self.data[offset..offset + length]
        } else {
            &[]
        }
    }

    /// Inserts a target for a node.
    ///
    /// If the node doesn't exist, creates a new entry.
    /// If the node exists, may need to relocate to a larger slot.
    pub fn insert(&mut self, node_id: u64, target: u64) {
        if let Some(&(offset, length)) = self.index.get(&node_id) {
            // Check if target already exists
            let slice = &self.data[offset..offset + length];
            if slice.contains(&target) {
                return;
            }

            // Need to add target - try to extend in place or relocate
            let new_length = length + 1;

            // Check if we can extend in place (next slot is free or end of data)
            let can_extend = offset + length == self.data.len()
                || self.try_merge_adjacent_free(offset + length, 1);

            if can_extend && offset + length == self.data.len() {
                // Extend at end
                self.data.push(target);
                self.index.insert(node_id, (offset, new_length));
            } else if can_extend {
                // Extended into adjacent free slot
                self.data[offset + length] = target;
                self.index.insert(node_id, (offset, new_length));
            } else {
                // Need to relocate
                let old_data: Vec<u64> = self.data[offset..offset + length].to_vec();
                self.mark_free(offset, length);

                // Find or allocate new slot
                let new_offset = self.allocate_slot(new_length);
                for (i, &val) in old_data.iter().enumerate() {
                    self.data[new_offset + i] = val;
                }
                self.data[new_offset + length] = target;
                self.index.insert(node_id, (new_offset, new_length));
            }
        } else {
            // New node - allocate slot
            let offset = self.allocate_slot(1);
            self.data[offset] = target;
            self.index.insert(node_id, (offset, 1));
        }

        // Check for compaction
        self.maybe_compact();
    }

    /// Removes a target from a node.
    ///
    /// Returns true if the target was present and removed.
    pub fn remove(&mut self, node_id: u64, target: u64) -> bool {
        if let Some(&(offset, length)) = self.index.get(&node_id) {
            let slice = &self.data[offset..offset + length];
            if let Some(pos) = slice.iter().position(|&t| t == target) {
                if length == 1 {
                    // Remove entire entry
                    self.mark_free(offset, length);
                    self.index.remove(&node_id);
                } else {
                    // Swap-remove within the slice
                    self.data[offset + pos] = self.data[offset + length - 1];
                    self.index.insert(node_id, (offset, length - 1));
                    // Mark the freed slot
                    self.free_slots.push((offset + length - 1, 1));
                    self.free_bytes += 1;
                }
                return true;
            }
        }
        false
    }

    /// Removes all neighbors for a node.
    pub fn remove_node(&mut self, node_id: u64) {
        if let Some((offset, length)) = self.index.remove(&node_id) {
            self.mark_free(offset, length);
        }
    }

    /// Returns the number of nodes in the index.
    #[must_use]
    pub fn node_count(&self) -> usize {
        self.index.len()
    }

    /// Returns the total number of edges stored.
    #[must_use]
    pub fn edge_count(&self) -> usize {
        self.index.values().map(|(_, len)| len).sum()
    }

    /// Returns the fragmentation ratio (0.0 to 1.0).
    #[must_use]
    pub fn fragmentation(&self) -> f64 {
        if self.data.is_empty() {
            0.0
        } else {
            self.free_bytes as f64 / self.data.len() as f64
        }
    }

    /// Compacts the index, eliminating fragmentation.
    pub fn compact(&mut self) {
        if self.free_bytes == 0 {
            return;
        }

        // Collect all node data
        let entries: Vec<(u64, Vec<u64>)> = self
            .index
            .iter()
            .map(|(&node_id, &(offset, length))| {
                (node_id, self.data[offset..offset + length].to_vec())
            })
            .collect();

        // Clear and rebuild
        self.data.clear();
        self.index.clear();
        self.free_slots.clear();
        self.free_bytes = 0;

        for (node_id, neighbors) in entries {
            let offset = self.data.len();
            let length = neighbors.len();
            self.data.extend(neighbors);
            self.index.insert(node_id, (offset, length));
        }
    }

    /// Checks if a node has a specific target.
    #[must_use]
    pub fn contains(&self, node_id: u64, target: u64) -> bool {
        self.get_neighbors(node_id).contains(&target)
    }

    /// Returns the number of neighbors for a node.
    #[must_use]
    pub fn neighbor_count(&self, node_id: u64) -> usize {
        self.index.get(&node_id).map_or(0, |(_, len)| *len)
    }

    fn allocate_slot(&mut self, needed: usize) -> usize {
        // First-fit allocation from free list
        for i in 0..self.free_slots.len() {
            let (offset, length) = self.free_slots[i];
            if length >= needed {
                if length > needed {
                    // Split the slot
                    self.free_slots[i] = (offset + needed, length - needed);
                } else {
                    // Exact fit
                    self.free_slots.swap_remove(i);
                }
                self.free_bytes = self.free_bytes.saturating_sub(needed);
                return offset;
            }
        }

        // No suitable free slot, append to end
        let offset = self.data.len();
        self.data.resize(self.data.len() + needed, 0);
        offset
    }

    fn mark_free(&mut self, offset: usize, length: usize) {
        self.free_slots.push((offset, length));
        self.free_bytes += length;
        self.merge_adjacent_free_slots();
    }

    fn try_merge_adjacent_free(&mut self, offset: usize, needed: usize) -> bool {
        for i in 0..self.free_slots.len() {
            let (free_offset, free_length) = self.free_slots[i];
            if free_offset == offset && free_length >= needed {
                if free_length > needed {
                    self.free_slots[i] = (free_offset + needed, free_length - needed);
                } else {
                    self.free_slots.swap_remove(i);
                }
                self.free_bytes = self.free_bytes.saturating_sub(needed);
                return true;
            }
        }
        false
    }

    fn merge_adjacent_free_slots(&mut self) {
        if self.free_slots.len() < 2 {
            return;
        }

        // Sort by offset
        self.free_slots.sort_by_key(|(offset, _)| *offset);

        // Merge adjacent slots
        let mut i = 0;
        while i < self.free_slots.len() - 1 {
            let (offset1, len1) = self.free_slots[i];
            let (offset2, len2) = self.free_slots[i + 1];

            if offset1 + len1 == offset2 {
                // Merge
                self.free_slots[i] = (offset1, len1 + len2);
                self.free_slots.remove(i + 1);
            } else {
                i += 1;
            }
        }
    }

    fn maybe_compact(&mut self) {
        if self.fragmentation() > FRAGMENTATION_THRESHOLD {
            self.compact();
        }
    }
}

impl Default for ClusteredIndex {
    fn default() -> Self {
        Self::new()
    }
}

/// Wrapper to implement `EdgeIndex` trait for `ClusteredIndex` per node.
#[derive(Debug)]
pub struct ClusteredEdgeIndex<'a> {
    index: &'a mut ClusteredIndex,
    node_id: u64,
}

impl<'a> ClusteredEdgeIndex<'a> {
    /// Creates a new edge index view for a specific node.
    pub fn new(index: &'a mut ClusteredIndex, node_id: u64) -> Self {
        Self { index, node_id }
    }
}

impl EdgeIndex for ClusteredEdgeIndex<'_> {
    fn insert(&mut self, target: u64) {
        self.index.insert(self.node_id, target);
    }

    fn remove(&mut self, target: u64) -> bool {
        self.index.remove(self.node_id, target)
    }

    fn contains(&self, target: u64) -> bool {
        self.index.contains(self.node_id, target)
    }

    fn targets(&self) -> Vec<u64> {
        self.index.get_neighbors(self.node_id).to_vec()
    }

    fn len(&self) -> usize {
        self.index.neighbor_count(self.node_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clustered_index_basic() {
        let mut index = ClusteredIndex::new();

        index.insert(1, 10);
        index.insert(1, 20);
        index.insert(1, 30);

        assert_eq!(index.get_neighbors(1).len(), 3);
        assert!(index.contains(1, 10));
        assert!(index.contains(1, 20));
        assert!(index.contains(1, 30));
    }

    #[test]
    fn test_clustered_index_multiple_nodes() {
        let mut index = ClusteredIndex::new();

        index.insert(1, 10);
        index.insert(1, 20);
        index.insert(2, 100);
        index.insert(2, 200);
        index.insert(3, 1000);

        assert_eq!(index.node_count(), 3);
        assert_eq!(index.edge_count(), 5);
        assert_eq!(index.neighbor_count(1), 2);
        assert_eq!(index.neighbor_count(2), 2);
        assert_eq!(index.neighbor_count(3), 1);
    }

    #[test]
    fn test_clustered_index_no_duplicates() {
        let mut index = ClusteredIndex::new();

        index.insert(1, 10);
        index.insert(1, 10);
        index.insert(1, 10);

        assert_eq!(index.neighbor_count(1), 1);
    }

    #[test]
    fn test_clustered_index_remove() {
        let mut index = ClusteredIndex::new();

        index.insert(1, 10);
        index.insert(1, 20);
        index.insert(1, 30);

        assert!(index.remove(1, 20));
        assert!(!index.contains(1, 20));
        assert_eq!(index.neighbor_count(1), 2);

        assert!(!index.remove(1, 99)); // Not present
    }

    #[test]
    fn test_clustered_index_remove_node() {
        let mut index = ClusteredIndex::new();

        index.insert(1, 10);
        index.insert(1, 20);
        index.insert(2, 100);

        index.remove_node(1);

        assert_eq!(index.node_count(), 1);
        assert_eq!(index.neighbor_count(1), 0);
        assert_eq!(index.neighbor_count(2), 1);
    }

    #[test]
    fn test_clustered_index_compaction() {
        let mut index = ClusteredIndex::new();

        // Create some data
        for i in 0..10 {
            for j in 0..5 {
                index.insert(i, j * 100);
            }
        }

        // Remove some to create fragmentation
        for i in 0..5 {
            index.remove_node(i);
        }

        let frag_before = index.fragmentation();
        assert!(frag_before > 0.0);

        index.compact();

        assert!(index.fragmentation().abs() < f64::EPSILON);
        assert_eq!(index.node_count(), 5);
    }

    #[test]
    fn test_clustered_index_slot_reuse() {
        let mut index = ClusteredIndex::new();

        // Fill some data
        index.insert(1, 10);
        index.insert(1, 20);

        // Remove and add - should reuse slots
        index.remove_node(1);
        index.insert(2, 100);

        assert_eq!(index.node_count(), 1);
        assert!(index.contains(2, 100));
    }
}
