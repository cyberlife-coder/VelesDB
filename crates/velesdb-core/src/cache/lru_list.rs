//! Single-threaded LRU recency list with O(1) operations.
//!
//! A hash map from key to slot, over a slab of nodes linked into a doubly
//! linked list ordered from least (`head`) to most (`tail`) recently used:
//! the classic LRU construction. Every operation is one hash lookup plus a
//! constant number of link updates, where an insertion-ordered map has to
//! shift every later entry to move or remove one. The slab stays dense: a
//! removal moves the last node into the freed slot and repoints its two
//! neighbours and its index entry, so it never outgrows the capacity.
//!
//! [`LruCache`](super::LruCache) wraps it in a lock and adds statistics.

use std::collections::HashMap;
use std::hash::Hash;
use std::ops::{Index, IndexMut};

/// The "no node" link.
const NIL: usize = usize::MAX;

struct Node<K, V> {
    key: K,
    value: V,
    prev: usize,
    next: usize,
}

/// The node slab: a `Vec` whose element accesses test builds count, the
/// structural probe of the O(1) guards in `lru_optimization_tests`. Every
/// node read or write goes through `Index`/`IndexMut`, so a walk over the
/// list would show up in the count as surely as in the running time.
struct Slab<T> {
    nodes: Vec<T>,
    #[cfg(test)]
    accesses: std::sync::atomic::AtomicU64,
}

impl<T> Slab<T> {
    fn with_capacity(capacity: usize) -> Self {
        Self {
            nodes: Vec::with_capacity(capacity),
            #[cfg(test)]
            accesses: std::sync::atomic::AtomicU64::new(0),
        }
    }

    fn len(&self) -> usize {
        self.nodes.len()
    }

    fn push(&mut self, node: T) {
        self.touch();
        self.nodes.push(node);
    }

    fn swap_remove(&mut self, slot: usize) -> T {
        self.touch();
        self.nodes.swap_remove(slot)
    }

    fn clear(&mut self) {
        self.nodes.clear();
    }

    /// Counts one node access in test builds; compiles to nothing otherwise.
    #[inline]
    #[cfg_attr(not(test), allow(clippy::unused_self))]
    fn touch(&self) {
        #[cfg(test)]
        self.accesses
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
}

impl<T> Index<usize> for Slab<T> {
    type Output = T;

    fn index(&self, slot: usize) -> &T {
        self.touch();
        &self.nodes[slot]
    }
}

impl<T> IndexMut<usize> for Slab<T> {
    fn index_mut(&mut self, slot: usize) -> &mut T {
        self.touch();
        &mut self.nodes[slot]
    }
}

/// Fixed-capacity map that keeps its entries in recency order.
pub(super) struct LruList<K, V> {
    capacity: usize,
    index: HashMap<K, usize>,
    /// Dense: every slot holds a linked node.
    slots: Slab<Node<K, V>>,
    /// Least recently used node, or `NIL` when empty.
    head: usize,
    /// Most recently used node, or `NIL` when empty.
    tail: usize,
}

impl<K: Hash + Eq + Clone, V> LruList<K, V> {
    pub(super) fn new(capacity: usize) -> Self {
        Self {
            capacity,
            index: HashMap::with_capacity(capacity),
            slots: Slab::with_capacity(capacity),
            head: NIL,
            tail: NIL,
        }
    }

    pub(super) fn len(&self) -> usize {
        self.index.len()
    }

    pub(super) fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    /// The value for `key`, which becomes the most recently used entry.
    pub(super) fn get(&mut self, key: &K) -> Option<&V> {
        let slot = *self.index.get(key)?;
        self.move_to_tail(slot);
        Some(&self.slots[slot].value)
    }

    /// The value for `key`, leaving the recency order unchanged.
    pub(super) fn peek(&self, key: &K) -> Option<&V> {
        self.index.get(key).map(|&slot| &self.slots[slot].value)
    }

    /// Inserts or replaces `key` as the most recently used entry.
    ///
    /// Returns the entry this displaced: the least recently used one when a
    /// new key meets a full list, or the entry itself at capacity 0, which
    /// stores nothing. Replacing an existing key's value displaces nothing.
    pub(super) fn insert(&mut self, key: K, value: V) -> Option<(K, V)> {
        if self.capacity == 0 {
            return Some((key, value));
        }
        if let Some(&slot) = self.index.get(&key) {
            self.slots[slot].value = value;
            self.move_to_tail(slot);
            return None;
        }
        let evicted = (self.len() >= self.capacity)
            .then(|| self.remove_slot(self.head))
            .map(|node| (node.key, node.value));
        let slot = self.slots.len();
        self.slots.push(Node {
            key: key.clone(),
            value,
            prev: NIL,
            next: NIL,
        });
        self.index.insert(key, slot);
        self.link_at_tail(slot);
        evicted
    }

    /// Removes `key`, keeping the order of the remaining entries.
    pub(super) fn remove(&mut self, key: &K) -> Option<V> {
        let slot = *self.index.get(key)?;
        Some(self.remove_slot(slot).value)
    }

    pub(super) fn clear(&mut self) {
        self.index.clear();
        self.slots.clear();
        self.head = NIL;
        self.tail = NIL;
    }

    /// Node accesses so far: the structural complexity probe.
    #[cfg(test)]
    pub(super) fn nodes_touched(&self) -> u64 {
        self.slots
            .accesses
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Keys from least to most recently used.
    #[cfg(test)]
    pub(super) fn keys_lru_to_mru(&self) -> Vec<K> {
        let mut keys = Vec::with_capacity(self.len());
        let mut slot = self.head;
        while slot != NIL {
            let node = &self.slots[slot];
            keys.push(node.key.clone());
            slot = node.next;
        }
        keys
    }

    /// Unlinks and removes the node in `slot`, and drops its index entry.
    ///
    /// `swap_remove` keeps the slab dense by moving the last node into
    /// `slot`; that node's neighbours and index entry are repointed to it.
    fn remove_slot(&mut self, slot: usize) -> Node<K, V> {
        self.unlink(slot);
        let node = self.slots.swap_remove(slot);
        self.index.remove(&node.key);
        if slot < self.slots.len() {
            self.repoint_moved_node(slot);
        }
        node
    }

    /// Repoints the links and the index entry of the node `swap_remove` just
    /// moved into `slot` from the end of the slab.
    fn repoint_moved_node(&mut self, slot: usize) {
        let Node { prev, next, .. } = self.slots[slot];
        if prev == NIL {
            self.head = slot;
        } else {
            self.slots[prev].next = slot;
        }
        if next == NIL {
            self.tail = slot;
        } else {
            self.slots[next].prev = slot;
        }
        if let Some(entry) = self.index.get_mut(&self.slots[slot].key) {
            *entry = slot;
        }
    }

    fn move_to_tail(&mut self, slot: usize) {
        if slot != self.tail {
            self.unlink(slot);
            self.link_at_tail(slot);
        }
    }

    fn unlink(&mut self, slot: usize) {
        let Node { prev, next, .. } = self.slots[slot];
        if prev == NIL {
            self.head = next;
        } else {
            self.slots[prev].next = next;
        }
        if next == NIL {
            self.tail = prev;
        } else {
            self.slots[next].prev = prev;
        }
    }

    fn link_at_tail(&mut self, slot: usize) {
        let old_tail = self.tail;
        let node = &mut self.slots[slot];
        node.prev = old_tail;
        node.next = NIL;
        if old_tail == NIL {
            self.head = slot;
        } else {
            self.slots[old_tail].next = slot;
        }
        self.tail = slot;
    }
}
