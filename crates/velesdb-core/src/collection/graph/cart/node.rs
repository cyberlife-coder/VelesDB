//! C-ART node types and operations.
//!
//! Implements the internal node variants (Node4, Node16, Node48, Node256, Leaf)
//! with search, insert, remove, and growth operations.

// SAFETY: Numeric casts in C-ART node operations are intentional:
// - usize->u8 for child indices: C-ART nodes have max 256 children, indices fit in u8
// - Node types enforce size limits (Node4=4, Node16=16, Node48=48, Node256=256)
// - All index values are validated against node capacity before casting
#![allow(clippy::cast_possible_truncation)]

/// Node variants for the Compressed Adaptive Radix Tree.
// Reason: Node256 is larger than other variants by design for high-degree vertices
#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum CARTNode {
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

impl CARTNode {
    /// Creates a new leaf node with a single entry.
    pub(crate) fn new_leaf(value: u64) -> Self {
        let mut entries = Vec::with_capacity(super::MAX_LEAF_ENTRIES);
        entries.push(value);
        Self::Leaf {
            entries,
            prefix: Vec::new(),
        }
    }

    /// Checks if this node is empty.
    pub(crate) fn is_empty(&self) -> bool {
        match self {
            Self::Leaf { entries, .. } => entries.is_empty(),
            Self::Node4 { num_children, .. }
            | Self::Node16 { num_children, .. }
            | Self::Node48 { num_children, .. } => *num_children == 0,
            Self::Node256 { num_children, .. } => *num_children == 0,
        }
    }

    /// Searches for a value in the subtree.
    pub(crate) fn search(&self, key: &[u8], value: u64) -> bool {
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
    pub(crate) fn insert(&mut self, key: &[u8], value: u64) -> bool {
        match self {
            Self::Leaf { entries, .. } => {
                // Binary search for insertion point
                match entries.binary_search(&value) {
                    Ok(_) => false, // Already exists
                    Err(pos) => {
                        // Leaf stores sorted local bucket; internal-node growth handles branching.
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
    pub(crate) fn remove(&mut self, key: &[u8], value: u64) -> bool {
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
    pub(crate) fn collect_all(&self, result: &mut Vec<u64>) {
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
                    num_children: u16::from(*num_children),
                    children: new_children,
                }
            }
            _ => self.clone(),
        }
    }
}
