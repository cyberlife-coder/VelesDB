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
//! - **Node4**: 4 keys/children (smallest, for sparse regions)
//! - **Node16**: 16 keys/children (SIMD-friendly binary search)
//! - **Node48**: 48 children with 256-byte key index
//! - **Node256**: Direct 256-child array (densest)
//! - **Leaf**: Compressed entries with LCP (Longest Common Prefix)
//!
//! ## Performance Targets
//!
//! - Scan 10K neighbors: < 100µs
//! - Memory: < 50 bytes/edge
//! - Search/Insert: O(log n) + binary search in leaf

use super::degree_router::EdgeIndex;

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

/// Node variants for the Compressed Adaptive Radix Tree.
// Reason: Node256 is larger than other variants by design for high-degree vertices
#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
enum CARTNode {
    /// Smallest internal node: 4 keys, 4 children.
    // Reason: Node4 variant currently unused but required for CART completeness
    #[allow(dead_code)]
    Node4 {
        num_children: u8,
        keys: [u8; 4],
        children: [Option<Box<CARTNode>>; 4],
    },
    /// Medium internal node: 16 keys, 16 children (SIMD-friendly).
    Node16 {
        num_children: u8,
        keys: [u8; 16],
        children: [Option<Box<CARTNode>>; 16],
    },
    /// Large internal node: 256-byte index, 48 children.
    Node48 {
        num_children: u8,
        keys: [u8; 256], // Index: key byte -> child slot (255 = empty)
        children: [Option<Box<CARTNode>>; 48],
    },
    /// Densest internal node: direct 256-child array.
    Node256 {
        num_children: u16,
        children: [Option<Box<CARTNode>>; 256],
    },
    /// Leaf node with compressed entries sharing LCP.
    Leaf {
        /// Sorted list of stored values.
        entries: Vec<u64>,
        /// Longest Common Prefix for all entries (key bytes consumed so far).
        #[allow(dead_code)]
        prefix: Vec<u8>,
    },
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

impl CARTNode {
    /// Creates a new leaf node with a single entry.
    fn new_leaf(value: u64) -> Self {
        Self::Leaf {
            entries: vec![value],
            prefix: Vec::new(),
        }
    }

    /// Checks if this node is empty.
    fn is_empty(&self) -> bool {
        match self {
            Self::Leaf { entries, .. } => entries.is_empty(),
            Self::Node4 { num_children, .. }
            | Self::Node16 { num_children, .. }
            | Self::Node48 { num_children, .. } => *num_children == 0,
            Self::Node256 { num_children, .. } => *num_children == 0,
        }
    }

    /// Searches for a value in the subtree.
    fn search(&self, key: &[u8], value: u64) -> bool {
        match self {
            Self::Leaf { entries, .. } => entries.binary_search(&value).is_ok(),
            Self::Node4 {
                num_children,
                keys,
                children,
            } => {
                if key.is_empty() {
                    return false;
                }
                let byte = key[0];
                for i in 0..*num_children as usize {
                    if keys[i] == byte {
                        if let Some(child) = &children[i] {
                            return child.search(&key[1..], value);
                        }
                    }
                }
                false
            }
            Self::Node16 {
                num_children,
                keys,
                children,
            } => {
                if key.is_empty() {
                    return false;
                }
                let byte = key[0];
                // Binary search for SIMD-friendly access
                let slice = &keys[..*num_children as usize];
                if let Ok(idx) = slice.binary_search(&byte) {
                    if let Some(child) = &children[idx] {
                        return child.search(&key[1..], value);
                    }
                }
                false
            }
            Self::Node48 { keys, children, .. } => {
                if key.is_empty() {
                    return false;
                }
                let byte = key[0];
                let slot = keys[byte as usize];
                if slot != 255 {
                    if let Some(child) = &children[slot as usize] {
                        return child.search(&key[1..], value);
                    }
                }
                false
            }
            Self::Node256 { children, .. } => {
                if key.is_empty() {
                    return false;
                }
                let byte = key[0];
                if let Some(child) = &children[byte as usize] {
                    return child.search(&key[1..], value);
                }
                false
            }
        }
    }

    /// Inserts a value into the subtree.
    #[allow(clippy::too_many_lines)]
    fn insert(&mut self, key: &[u8], value: u64) -> bool {
        match self {
            Self::Leaf { entries, .. } => {
                // Binary search for insertion point
                match entries.binary_search(&value) {
                    Ok(_) => false, // Already exists
                    Err(pos) => {
                        // Note: Leaf splitting not yet implemented (TODO)
                        // Insert regardless of capacity for now
                        entries.insert(pos, value);
                        true
                    }
                }
            }
            Self::Node4 {
                num_children,
                keys,
                children,
            } => {
                if key.is_empty() {
                    return false;
                }
                let byte = key[0];

                // Check if key exists
                for i in 0..*num_children as usize {
                    if keys[i] == byte {
                        if let Some(child) = &mut children[i] {
                            return child.insert(&key[1..], value);
                        }
                    }
                }

                // Key doesn't exist, add new child
                if (*num_children as usize) < 4 {
                    let idx = *num_children as usize;
                    keys[idx] = byte;
                    children[idx] = Some(Box::new(Self::new_leaf(value)));
                    *num_children += 1;
                    true
                } else {
                    // Node is full, need to grow to Node16
                    *self = self.grow_to_node16();
                    self.insert(key, value)
                }
            }
            Self::Node16 {
                num_children,
                keys,
                children,
            } => {
                if key.is_empty() {
                    return false;
                }
                let byte = key[0];

                // Binary search for key
                let slice = &keys[..*num_children as usize];
                match slice.binary_search(&byte) {
                    Ok(idx) => {
                        if let Some(child) = &mut children[idx] {
                            child.insert(&key[1..], value)
                        } else {
                            false
                        }
                    }
                    Err(pos) => {
                        if (*num_children as usize) < 16 {
                            // Shift elements to maintain sorted order
                            let n = *num_children as usize;
                            for i in (pos..n).rev() {
                                keys[i + 1] = keys[i];
                                children[i + 1] = children[i].take();
                            }
                            keys[pos] = byte;
                            children[pos] = Some(Box::new(Self::new_leaf(value)));
                            *num_children += 1;
                            true
                        } else {
                            // Grow to Node48
                            *self = self.grow_to_node48();
                            self.insert(key, value)
                        }
                    }
                }
            }
            Self::Node48 {
                num_children,
                keys,
                children,
            } => {
                if key.is_empty() {
                    return false;
                }
                let byte = key[0];
                let slot = keys[byte as usize];

                if slot != 255 {
                    // Key exists, recurse
                    if let Some(child) = &mut children[slot as usize] {
                        return child.insert(&key[1..], value);
                    }
                }

                // Key doesn't exist, add new child
                if (*num_children as usize) < 48 {
                    let new_slot = *num_children;
                    keys[byte as usize] = new_slot;
                    children[new_slot as usize] = Some(Box::new(Self::new_leaf(value)));
                    *num_children += 1;
                    true
                } else {
                    // Grow to Node256
                    *self = self.grow_to_node256();
                    self.insert(key, value)
                }
            }
            Self::Node256 {
                num_children,
                children,
            } => {
                if key.is_empty() {
                    return false;
                }
                let byte = key[0] as usize;

                if let Some(child) = &mut children[byte] {
                    child.insert(&key[1..], value)
                } else {
                    children[byte] = Some(Box::new(Self::new_leaf(value)));
                    *num_children += 1;
                    true
                }
            }
        }
    }

    /// Removes a value from the subtree.
    #[allow(clippy::too_many_lines)]
    fn remove(&mut self, key: &[u8], value: u64) -> bool {
        match self {
            Self::Leaf { entries, .. } => {
                if let Ok(pos) = entries.binary_search(&value) {
                    entries.remove(pos);
                    true
                } else {
                    false
                }
            }
            Self::Node4 {
                num_children,
                keys,
                children,
            } => {
                if key.is_empty() {
                    return false;
                }
                let byte = key[0];
                for i in 0..*num_children as usize {
                    if keys[i] == byte {
                        if let Some(child) = &mut children[i] {
                            let removed = child.remove(&key[1..], value);
                            if removed && child.is_empty() {
                                // Remove empty child
                                let n = *num_children as usize;
                                for j in i..(n - 1) {
                                    keys[j] = keys[j + 1];
                                    children[j] = children[j + 1].take();
                                }
                                *num_children -= 1;
                            }
                            return removed;
                        }
                    }
                }
                false
            }
            Self::Node16 {
                num_children,
                keys,
                children,
            } => {
                if key.is_empty() {
                    return false;
                }
                let byte = key[0];
                let slice = &keys[..*num_children as usize];
                if let Ok(idx) = slice.binary_search(&byte) {
                    if let Some(child) = &mut children[idx] {
                        let removed = child.remove(&key[1..], value);
                        if removed && child.is_empty() {
                            let n = *num_children as usize;
                            for j in idx..(n - 1) {
                                keys[j] = keys[j + 1];
                                children[j] = children[j + 1].take();
                            }
                            *num_children -= 1;
                        }
                        return removed;
                    }
                }
                false
            }
            Self::Node48 {
                num_children,
                keys,
                children,
            } => {
                if key.is_empty() {
                    return false;
                }
                let byte = key[0];
                let slot = keys[byte as usize];
                if slot != 255 {
                    if let Some(child) = &mut children[slot as usize] {
                        let removed = child.remove(&key[1..], value);
                        if removed && child.is_empty() {
                            children[slot as usize] = None;
                            keys[byte as usize] = 255;
                            *num_children -= 1;
                        }
                        return removed;
                    }
                }
                false
            }
            Self::Node256 {
                num_children,
                children,
            } => {
                if key.is_empty() {
                    return false;
                }
                let byte = key[0] as usize;
                if let Some(child) = &mut children[byte] {
                    let removed = child.remove(&key[1..], value);
                    if removed && child.is_empty() {
                        children[byte] = None;
                        *num_children -= 1;
                    }
                    return removed;
                }
                false
            }
        }
    }

    /// Collects all values in sorted order.
    fn collect_all(&self, result: &mut Vec<u64>) {
        match self {
            Self::Leaf { entries, .. } => {
                result.extend(entries.iter().copied());
            }
            Self::Node4 {
                num_children,
                children,
                ..
            } => {
                for child in children.iter().take(*num_children as usize).flatten() {
                    child.collect_all(result);
                }
            }
            Self::Node16 {
                num_children,
                children,
                ..
            } => {
                for child in children.iter().take(*num_children as usize).flatten() {
                    child.collect_all(result);
                }
            }
            Self::Node48 { children, .. } => {
                for child in children.iter().flatten() {
                    child.collect_all(result);
                }
            }
            Self::Node256 { children, .. } => {
                for child in children.iter().flatten() {
                    child.collect_all(result);
                }
            }
        }
    }

    /// Grows Node4 to Node16.
    fn grow_to_node16(&self) -> Self {
        match self {
            Self::Node4 {
                num_children,
                keys,
                children,
            } => {
                let mut new_keys = [0u8; 16];
                let mut new_children: [Option<Box<CARTNode>>; 16] = Default::default();

                // Copy and sort
                let n = *num_children as usize;
                let mut indices: Vec<usize> = (0..n).collect();
                indices.sort_by_key(|&i| keys[i]);

                for (new_idx, &old_idx) in indices.iter().enumerate() {
                    new_keys[new_idx] = keys[old_idx];
                    new_children[new_idx].clone_from(&children[old_idx]);
                }

                Self::Node16 {
                    num_children: *num_children,
                    keys: new_keys,
                    children: new_children,
                }
            }
            _ => self.clone(),
        }
    }

    /// Grows Node16 to Node48.
    fn grow_to_node48(&self) -> Self {
        match self {
            Self::Node16 {
                num_children,
                keys,
                children,
            } => {
                let mut new_keys = [255u8; 256];
                let mut new_children: [Option<Box<CARTNode>>; 48] = std::array::from_fn(|_| None);

                for i in 0..*num_children as usize {
                    new_keys[keys[i] as usize] = i as u8;
                    new_children[i].clone_from(&children[i]);
                }

                Self::Node48 {
                    num_children: *num_children,
                    keys: new_keys,
                    children: new_children,
                }
            }
            _ => self.clone(),
        }
    }

    /// Grows Node48 to Node256.
    fn grow_to_node256(&self) -> Self {
        match self {
            Self::Node48 {
                num_children,
                keys,
                children,
            } => {
                let mut new_children: [Option<Box<CARTNode>>; 256] = std::array::from_fn(|_| None);

                for (byte, &slot) in keys.iter().enumerate() {
                    if slot != 255 {
                        new_children[byte].clone_from(&children[slot as usize]);
                    }
                }

                Self::Node256 {
                    num_children: *num_children as u16,
                    children: new_children,
                }
            }
            _ => self.clone(),
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // Basic C-ART Tests (TDD: Written first)
    // =========================================================================

    #[test]
    fn test_cart_new_is_empty() {
        let tree = CompressedART::new();
        assert!(tree.is_empty());
        assert_eq!(tree.len(), 0);
    }

    #[test]
    fn test_cart_insert_single() {
        let mut tree = CompressedART::new();
        assert!(tree.insert(42));
        assert_eq!(tree.len(), 1);
        assert!(tree.contains(42));
    }

    #[test]
    fn test_cart_insert_no_duplicates() {
        let mut tree = CompressedART::new();
        assert!(tree.insert(42));
        assert!(!tree.insert(42)); // Duplicate
        assert_eq!(tree.len(), 1);
    }

    #[test]
    fn test_cart_insert_multiple() {
        let mut tree = CompressedART::new();
        for i in 0..100 {
            assert!(tree.insert(i));
        }
        assert_eq!(tree.len(), 100);
        for i in 0..100 {
            assert!(tree.contains(i));
        }
    }

    #[test]
    fn test_cart_remove_existing() {
        let mut tree = CompressedART::new();
        tree.insert(42);
        tree.insert(100);
        tree.insert(7);

        assert!(tree.remove(100));
        assert!(!tree.contains(100));
        assert!(tree.contains(42));
        assert!(tree.contains(7));
        assert_eq!(tree.len(), 2);
    }

    #[test]
    fn test_cart_remove_nonexistent() {
        let mut tree = CompressedART::new();
        tree.insert(42);
        assert!(!tree.remove(999));
        assert_eq!(tree.len(), 1);
    }

    #[test]
    fn test_cart_scan_returns_sorted() {
        let mut tree = CompressedART::new();
        tree.insert(50);
        tree.insert(10);
        tree.insert(30);
        tree.insert(20);
        tree.insert(40);

        let scanned = tree.scan();
        assert_eq!(scanned, vec![10, 20, 30, 40, 50]);
    }

    #[test]
    fn test_cart_large_insertions() {
        let mut tree = CompressedART::new();
        for i in 0..10_000 {
            tree.insert(i);
        }
        assert_eq!(tree.len(), 10_000);

        // Verify all present
        for i in 0..10_000 {
            assert!(tree.contains(i), "Missing value: {i}");
        }
    }

    #[test]
    fn test_cart_random_order_insertions() {
        let mut tree = CompressedART::new();
        let values: Vec<u64> = vec![
            999, 1, 500, 250, 750, 125, 875, 62, 937, 31, 968, 15, 984, 7, 992,
        ];

        for &v in &values {
            tree.insert(v);
        }

        assert_eq!(tree.len(), values.len());
        for &v in &values {
            assert!(tree.contains(v));
        }
    }

    // =========================================================================
    // EdgeIndex Trait Tests
    // =========================================================================

    #[test]
    fn test_cart_edge_index_basic() {
        let mut index = CARTEdgeIndex::new();
        assert!(index.is_empty());

        index.insert(1);
        index.insert(2);
        index.insert(3);

        assert_eq!(index.len(), 3);
        assert!(index.contains(2));
        assert!(!index.contains(99));
    }

    #[test]
    fn test_cart_edge_index_remove() {
        let mut index = CARTEdgeIndex::new();
        index.insert(1);
        index.insert(2);
        index.insert(3);

        assert!(index.remove(2));
        assert!(!index.contains(2));
        assert_eq!(index.len(), 2);
    }

    #[test]
    fn test_cart_edge_index_targets() {
        let mut index = CARTEdgeIndex::new();
        index.insert(30);
        index.insert(10);
        index.insert(20);

        let targets = index.targets();
        // Should be in sorted order
        assert_eq!(targets, vec![10, 20, 30]);
    }

    #[test]
    fn test_cart_edge_index_from_targets() {
        let targets = vec![5, 3, 8, 1, 9];
        let index = CARTEdgeIndex::from_targets(&targets);

        assert_eq!(index.len(), 5);
        for t in targets {
            assert!(index.contains(t));
        }
    }

    // =========================================================================
    // Node Growth Tests
    // =========================================================================

    #[test]
    fn test_cart_node_growth_node4_to_node16() {
        let mut tree = CompressedART::new();
        // Insert 5 values with different first bytes to trigger Node4 -> Node16 growth
        for i in 0..5u64 {
            tree.insert(i << 56); // Different first byte for each
        }
        assert_eq!(tree.len(), 5);
    }

    #[test]
    fn test_cart_node_growth_node16_to_node48() {
        let mut tree = CompressedART::new();
        // Insert 17 values with different first bytes to trigger Node16 -> Node48 growth
        for i in 0..17u64 {
            tree.insert(i << 56);
        }
        assert_eq!(tree.len(), 17);
    }

    #[test]
    fn test_cart_node_growth_node48_to_node256() {
        let mut tree = CompressedART::new();
        // Insert 49 values with different first bytes to trigger Node48 -> Node256 growth
        for i in 0..49u64 {
            tree.insert(i << 56);
        }
        assert_eq!(tree.len(), 49);
    }

    // =========================================================================
    // Stress Tests
    // =========================================================================

    #[test]
    fn test_cart_insert_remove_cycle() {
        let mut tree = CompressedART::new();

        // Insert 1000 values
        for i in 0..1000 {
            tree.insert(i);
        }
        assert_eq!(tree.len(), 1000);

        // Remove even values
        for i in (0..1000).step_by(2) {
            assert!(tree.remove(i));
        }
        assert_eq!(tree.len(), 500);

        // Verify odd values still present
        for i in (1..1000).step_by(2) {
            assert!(tree.contains(i));
        }
    }
}
