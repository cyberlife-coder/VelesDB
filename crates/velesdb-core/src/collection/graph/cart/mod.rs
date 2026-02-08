//! Compressed Adaptive Radix Tree (C-ART) for high-degree vertex storage.
//!
//! This module implements C-ART based on RapidStore (arXiv:2507.00839) for
//! efficient storage of large adjacency lists in graph databases.
//!
//! # EPIC-020 US-002: C-ART for High-Degree Vertices
//!
//! ## Design
//!
//! C-ART uses horizontal compression with leaf nodes holding up to 256 entries,
//! achieving >60% filling ratio (vs <4% for standard ART).
//!
//! ## Node Types
//!
//! - **Node16**: 16 keys/children (SIMD-friendly binary search)
//! - **Node48**: 48 children with 256-byte key index
//! - **Node256**: Direct 256-child array (densest)
//! - **Leaf**: Compressed entries with LCP (Longest Common Prefix)
//!
//! Note: Node4 was removed as unused dead code. Leaf splitting is not yet
//! implemented, so leaves grow unbounded. If adaptive splitting is needed,
//! re-implement Node4 with proper tests.
//!
//! ## Performance Targets
//!
//! - Scan 10K neighbors: < 100µs
//! - Memory: < 50 bytes/edge
//! - Search/Insert: O(log n) + binary search in leaf

// SAFETY: Numeric casts in C-ART node operations are intentional:
// - usize->u8 for child indices: C-ART nodes have max 256 children, indices fit in u8
// - Node types enforce size limits (Node16=16, Node48=48, Node256=256)
// - All index values are validated against node capacity before casting
#![allow(clippy::cast_possible_truncation)]

mod node;

#[cfg(test)]
mod tests;

use super::degree_router::EdgeIndex;
use node::CARTNode;

/// Maximum entries per leaf node (horizontal compression).
// Reason: MAX_LEAF_ENTRIES will be used for leaf splitting when CART implementation is complete
#[allow(dead_code)]
const MAX_LEAF_ENTRIES: usize = 256;

/// Compressed Adaptive Radix Tree for high-degree vertices.
///
/// Optimized for storing large sets of u64 neighbor IDs with:
/// - O(log n) search/insert/remove
/// - Cache-friendly leaf scanning
/// - Horizontal compression for high fill ratio
#[derive(Debug, Clone)]
pub struct CompressedART {
    root: Option<Box<CARTNode>>,
    len: usize,
}

impl Default for CompressedART {
    fn default() -> Self {
        Self::new()
    }
}

impl CompressedART {
    /// Creates a new empty C-ART.
    #[must_use]
    pub fn new() -> Self {
        Self { root: None, len: 0 }
    }

    /// Returns the number of entries in the tree.
    #[must_use]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns true if the tree is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Inserts a value into the tree.
    ///
    /// Returns `true` if the value was newly inserted.
    pub fn insert(&mut self, value: u64) -> bool {
        if self.contains(value) {
            return false;
        }

        match &mut self.root {
            None => {
                // First insertion: create a leaf
                self.root = Some(Box::new(CARTNode::new_leaf(value)));
                self.len = 1;
                true
            }
            Some(root) => {
                let key_bytes = value.to_be_bytes();
                if root.insert(&key_bytes, value) {
                    self.len += 1;
                    true
                } else {
                    false
                }
            }
        }
    }

    /// Checks if a value exists in the tree.
    #[must_use]
    pub fn contains(&self, value: u64) -> bool {
        match &self.root {
            None => false,
            Some(root) => {
                let key_bytes = value.to_be_bytes();
                root.search(&key_bytes, value)
            }
        }
    }

    /// Removes a value from the tree.
    ///
    /// Returns `true` if the value was present and removed.
    pub fn remove(&mut self, value: u64) -> bool {
        match &mut self.root {
            None => false,
            Some(root) => {
                let key_bytes = value.to_be_bytes();
                if root.remove(&key_bytes, value) {
                    self.len -= 1;
                    // Clean up empty root
                    if root.is_empty() {
                        self.root = None;
                    }
                    true
                } else {
                    false
                }
            }
        }
    }

    /// Returns all values in sorted order (DFS traversal).
    #[must_use]
    pub fn scan(&self) -> Vec<u64> {
        let mut result = Vec::with_capacity(self.len);
        if let Some(root) = &self.root {
            root.collect_all(&mut result);
        }
        result
    }

    /// Returns an iterator over all values.
    pub fn iter(&self) -> impl Iterator<Item = u64> + '_ {
        self.scan().into_iter()
    }
}

/// C-ART implementation of EdgeIndex for integration with DegreeRouter.
#[derive(Debug, Clone, Default)]
pub struct CARTEdgeIndex {
    tree: CompressedART,
}

impl CARTEdgeIndex {
    /// Creates a new empty C-ART edge index.
    #[must_use]
    pub fn new() -> Self {
        Self {
            tree: CompressedART::new(),
        }
    }

    /// Creates from an existing vector of targets.
    #[must_use]
    pub fn from_targets(targets: &[u64]) -> Self {
        let mut tree = CompressedART::new();
        for &target in targets {
            tree.insert(target);
        }
        Self { tree }
    }
}

impl EdgeIndex for CARTEdgeIndex {
    fn insert(&mut self, target: u64) {
        self.tree.insert(target);
    }

    fn remove(&mut self, target: u64) -> bool {
        self.tree.remove(target)
    }

    fn contains(&self, target: u64) -> bool {
        self.tree.contains(target)
    }

    fn targets(&self) -> Vec<u64> {
        self.tree.scan()
    }

    fn len(&self) -> usize {
        self.tree.len()
    }
}
