//! Single-threaded LRU recency list with O(1) operations.
//!
//! A hash map from key to slot, over a slab of nodes linked into a doubly
//! linked list ordered from least (`head`) to most (`tail`) recently used:
//! the classic LRU construction. Every operation is one hash lookup plus a
//! constant number of link updates, where an insertion-ordered map has to
//! shift every later entry to move or remove one. Slots freed by removal
//! or eviction are reused, so the slab never outgrows the capacity.
//!
//! [`LruCache`](super::LruCache) wraps it in a lock and adds statistics.

use std::collections::HashMap;
use std::hash::Hash;

/// The "no node" link.
const NIL: usize = usize::MAX;

struct Node<K, V> {
    key: K,
    value: V,
    prev: usize,
    next: usize,
}

/// Fixed-capacity map that keeps its entries in recency order.
pub(super) struct LruList<K, V> {
    capacity: usize,
    index: HashMap<K, usize>,
    /// `None` marks a free slot, listed in `free`.
    slots: Vec<Option<Node<K, V>>>,
    free: Vec<usize>,
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
            slots: Vec::with_capacity(capacity),
            free: Vec::new(),
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
        Some(&self.node(slot).value)
    }

    /// The value for `key`, leaving the recency order unchanged.
    pub(super) fn peek(&self, key: &K) -> Option<&V> {
        self.index.get(key).map(|&slot| &self.node(slot).value)
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
            self.node_mut(slot).value = value;
            self.move_to_tail(slot);
            return None;
        }
        let evicted = (self.len() >= self.capacity)
            .then(|| self.remove_slot(self.head))
            .map(|node| (node.key, node.value));
        let slot = self.allocate(Node {
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
        self.free.clear();
        self.head = NIL;
        self.tail = NIL;
    }

    /// Keys from least to most recently used.
    #[cfg(test)]
    pub(super) fn keys_lru_to_mru(&self) -> Vec<K> {
        let mut keys = Vec::with_capacity(self.len());
        let mut slot = self.head;
        while slot != NIL {
            let node = self.node(slot);
            keys.push(node.key.clone());
            slot = node.next;
        }
        keys
    }

    fn node(&self, slot: usize) -> &Node<K, V> {
        self.slots[slot].as_ref().expect("linked slot holds a node")
    }

    fn node_mut(&mut self, slot: usize) -> &mut Node<K, V> {
        self.slots[slot].as_mut().expect("linked slot holds a node")
    }

    fn allocate(&mut self, node: Node<K, V>) -> usize {
        if let Some(slot) = self.free.pop() {
            self.slots[slot] = Some(node);
            slot
        } else {
            self.slots.push(Some(node));
            self.slots.len() - 1
        }
    }

    /// Unlinks the node in `slot`, frees the slot and drops its index entry.
    fn remove_slot(&mut self, slot: usize) -> Node<K, V> {
        self.unlink(slot);
        let node = self.slots[slot].take().expect("linked slot holds a node");
        self.free.push(slot);
        self.index.remove(&node.key);
        node
    }

    fn move_to_tail(&mut self, slot: usize) {
        if slot != self.tail {
            self.unlink(slot);
            self.link_at_tail(slot);
        }
    }

    fn unlink(&mut self, slot: usize) {
        let Node { prev, next, .. } = *self.node(slot);
        if prev == NIL {
            self.head = next;
        } else {
            self.node_mut(prev).next = next;
        }
        if next == NIL {
            self.tail = prev;
        } else {
            self.node_mut(next).prev = prev;
        }
    }

    fn link_at_tail(&mut self, slot: usize) {
        let old_tail = self.tail;
        {
            let node = self.node_mut(slot);
            node.prev = old_tail;
            node.next = NIL;
        }
        if old_tail == NIL {
            self.head = slot;
        } else {
            self.node_mut(old_tail).next = slot;
        }
        self.tail = slot;
    }
}
