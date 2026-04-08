//! C-ART node implementation.
//!
//! Implements the internal node variants (Node4, Node16, Node48, Node256, Leaf)
//! with search, insert, remove, and collect operations.
//! Growth operations (Node4→Node16→Node48→Node256) are in `growth.rs`.

// SAFETY: Numeric casts in C-ART node operations are intentional:
// - usize->u8 for child indices: C-ART nodes have max 256 children, indices fit in u8
// - Node types enforce size limits (Node4=4, Node16=16, Node48=48, Node256=256)
// - All index values are validated against node capacity before casting
#![allow(clippy::cast_possible_truncation)]

mod growth;

#[cfg(test)]
mod growth_tests;

/// Node variants for the Compressed Adaptive Radix Tree.
// SAFETY: Node256 is larger than other variants by design for high-degree vertices
#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum CARTNode {
    /// Smallest internal node: 4 keys, 4 children.
    // SAFETY: Node4 variant currently unused but required for CART completeness
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

    /// Returns true if this node is empty.
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
                ..
            } => Self::search_node4(key, value, *num_children, *keys, children),
            Self::Node16 {
                num_children,
                keys,
                children,
                ..
            } => Self::search_node16(key, value, *num_children, keys, children),
            Self::Node48 { keys, children, .. } => Self::search_node48(key, value, keys, children),
            Self::Node256 { children, .. } => Self::search_node256(key, value, children),
        }
    }

    /// Searches for a value in a Node4 by linear scan of keys.
    fn search_node4(
        key: &[u8],
        value: u64,
        num_children: u8,
        keys: [u8; 4],
        children: &[Option<Box<CARTNode>>; 4],
    ) -> bool {
        if key.is_empty() {
            return false;
        }
        let byte = key[0];
        for i in 0..num_children as usize {
            if keys[i] == byte {
                if let Some(child) = &children[i] {
                    return child.search(&key[1..], value);
                }
            }
        }
        false
    }

    /// Searches for a value in a Node16 by binary search of sorted keys.
    fn search_node16(
        key: &[u8],
        value: u64,
        num_children: u8,
        keys: &[u8; 16],
        children: &[Option<Box<CARTNode>>; 16],
    ) -> bool {
        if key.is_empty() {
            return false;
        }
        let byte = key[0];
        let slice = &keys[..num_children as usize];
        if let Ok(idx) = slice.binary_search(&byte) {
            if let Some(child) = &children[idx] {
                return child.search(&key[1..], value);
            }
        }
        false
    }

    /// Searches for a value in a Node48 via the 256-byte index lookup.
    fn search_node48(
        key: &[u8],
        value: u64,
        keys: &[u8; 256],
        children: &[Option<Box<CARTNode>>; 48],
    ) -> bool {
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

    /// Searches for a value in a Node256 via direct array indexing.
    fn search_node256(key: &[u8], value: u64, children: &[Option<Box<CARTNode>>; 256]) -> bool {
        if key.is_empty() {
            return false;
        }
        let byte = key[0] as usize;
        if let Some(child) = &children[byte] {
            child.search(&key[1..], value)
        } else {
            false
        }
    }

    /// Inserts a value into the subtree.
    pub(crate) fn insert(&mut self, key: &[u8], value: u64) -> bool {
        match self {
            Self::Leaf { entries, .. } => Self::insert_into_leaf(entries, value),
            Self::Node4 { .. } => self.insert_node4(key, value),
            Self::Node16 { .. } => self.insert_node16(key, value),
            Self::Node48 { .. } => self.insert_node48(key, value),
            Self::Node256 { .. } => self.insert_node256(key, value),
        }
    }

    /// Inserts a value into a leaf node's sorted entry list.
    fn insert_into_leaf(entries: &mut Vec<u64>, value: u64) -> bool {
        match entries.binary_search(&value) {
            Ok(_) => false, // Already exists
            Err(pos) => {
                entries.insert(pos, value);
                true
            }
        }
    }

    /// Inserts into a Node4, growing to Node16 when full.
    fn insert_node4(&mut self, key: &[u8], value: u64) -> bool {
        let Self::Node4 {
            num_children,
            keys,
            children,
        } = self
        else {
            return false;
        };
        if key.is_empty() {
            return false;
        }
        let byte = key[0];

        for i in 0..*num_children as usize {
            if keys[i] == byte {
                if let Some(child) = &mut children[i] {
                    return child.insert(&key[1..], value);
                }
            }
        }

        if (*num_children as usize) < 4 {
            let idx = *num_children as usize;
            keys[idx] = byte;
            children[idx] = Some(Box::new(Self::new_leaf(value)));
            *num_children += 1;
            true
        } else {
            *self = self.grow_to_node16();
            self.insert(key, value)
        }
    }

    /// Inserts into a Node16, growing to Node48 when full.
    fn insert_node16(&mut self, key: &[u8], value: u64) -> bool {
        let Self::Node16 {
            num_children,
            keys,
            children,
        } = self
        else {
            return false;
        };
        if key.is_empty() {
            return false;
        }
        let byte = key[0];

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
                    Self::shift_and_insert(keys, children, num_children, pos, byte, value);
                    true
                } else {
                    *self = self.grow_to_node48();
                    self.insert(key, value)
                }
            }
        }
    }

    /// Shifts keys/children right and inserts a new child at `pos` in a Node16.
    fn shift_and_insert(
        keys: &mut [u8; 16],
        children: &mut [Option<Box<CARTNode>>; 16],
        num_children: &mut u8,
        pos: usize,
        byte: u8,
        value: u64,
    ) {
        let n = *num_children as usize;
        for i in (pos..n).rev() {
            keys[i + 1] = keys[i];
            children[i + 1] = children[i].take();
        }
        keys[pos] = byte;
        children[pos] = Some(Box::new(Self::new_leaf(value)));
        *num_children += 1;
    }

    /// Inserts into a Node48, growing to Node256 when full.
    fn insert_node48(&mut self, key: &[u8], value: u64) -> bool {
        let Self::Node48 {
            num_children,
            keys,
            children,
        } = self
        else {
            return false;
        };
        if key.is_empty() {
            return false;
        }
        let byte = key[0];
        let slot = keys[byte as usize];

        if slot != 255 {
            if let Some(child) = &mut children[slot as usize] {
                return child.insert(&key[1..], value);
            }
        }

        if (*num_children as usize) < 48 {
            let new_slot = *num_children;
            keys[byte as usize] = new_slot;
            children[new_slot as usize] = Some(Box::new(Self::new_leaf(value)));
            *num_children += 1;
            true
        } else {
            *self = self.grow_to_node256();
            self.insert(key, value)
        }
    }

    /// Inserts into a Node256 (densest node, no growth needed).
    fn insert_node256(&mut self, key: &[u8], value: u64) -> bool {
        let Self::Node256 {
            num_children,
            children,
        } = self
        else {
            return false;
        };
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
                let n = *num_children as usize;
                let idx = (0..n).find(|&i| keys[i] == byte);
                let Some(idx) = idx else { return false };
                Self::remove_from_child_compact(&key[1..], value, idx, num_children, keys, children)
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
                let Ok(idx) = slice.binary_search(&byte) else {
                    return false;
                };
                Self::remove_from_child_compact(&key[1..], value, idx, num_children, keys, children)
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

    /// Shared remove + compact logic for Node4 and Node16.
    ///
    /// Recurses into `children[idx]`, and if the child becomes empty after
    /// removal, shifts keys/children left to fill the gap.
    fn remove_from_child_compact(
        key_rest: &[u8],
        value: u64,
        idx: usize,
        num_children: &mut u8,
        keys: &mut [u8],
        children: &mut [Option<Box<CARTNode>>],
    ) -> bool {
        let Some(child) = &mut children[idx] else {
            return false;
        };
        let removed = child.remove(key_rest, value);
        if removed && child.is_empty() {
            children[idx] = None;
            let n = *num_children as usize;
            for j in idx..n.saturating_sub(1) {
                keys[j] = keys[j + 1];
                children[j] = children[j + 1].take();
            }
            *num_children -= 1;
        }
        removed
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
            } => Self::collect_children(children.iter(), *num_children as usize, result),
            Self::Node16 {
                num_children,
                children,
                ..
            } => Self::collect_children(children.iter(), *num_children as usize, result),
            Self::Node48 { children, .. } => {
                Self::collect_children(children.iter(), children.len(), result);
            }
            Self::Node256 { children, .. } => {
                Self::collect_children(children.iter(), children.len(), result);
            }
        }
    }

    /// Recursively collects values from up to `count` child slots.
    fn collect_children<'a>(
        children: impl Iterator<Item = &'a Option<Box<CARTNode>>>,
        count: usize,
        result: &mut Vec<u64>,
    ) {
        for child in children.take(count).flatten() {
            child.collect_all(result);
        }
    }
}
