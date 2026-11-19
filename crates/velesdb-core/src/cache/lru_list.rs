//! Bounded recency-ordered map with O(1) promote, evict and remove.
//!
//! Nodes live densely in a `Vec` and are chained by index into a doubly-linked
//! list (head = LRU, tail = MRU); a `HashMap` maps each key to its node index.
//! A new key arriving at capacity takes over the LRU node in place, so the
//! `Vec` is allocated once and never grows past capacity. Removal swaps the
//! last node into the freed slot and re-points its two neighbours and its map
//! entry. Every operation touches a constant number of nodes.

use std::collections::HashMap;
use std::hash::Hash;

/// Index sentinel for "no node" (list ends).
const NIL: usize = usize::MAX;

struct Node<K, V> {
    key: K,
    value: V,
    prev: usize,
    next: usize,
}

/// Single-threaded bounded LRU map; `LruCache` adds locking and stats.
pub(super) struct LruList<K, V> {
    capacity: usize,
    index: HashMap<K, usize>,
    nodes: Vec<Node<K, V>>,
    head: usize,
    tail: usize,
    /// Node accesses, the structural complexity probe of the O(1) tests.
    #[cfg(test)]
    touched: std::sync::atomic::AtomicU64,
}

impl<K: Hash + Eq + Clone, V> LruList<K, V> {
    pub(super) fn new(capacity: usize) -> Self {
        Self {
            capacity,
            index: HashMap::with_capacity(capacity),
            nodes: Vec::with_capacity(capacity),
            head: NIL,
            tail: NIL,
            #[cfg(test)]
            touched: std::sync::atomic::AtomicU64::new(0),
        }
    }

    pub(super) fn len(&self) -> usize {
        self.nodes.len()
    }

    pub(super) fn capacity(&self) -> usize {
        self.capacity
    }

    /// Value for `key` without changing its recency.
    pub(super) fn peek(&self, key: &K) -> Option<&V> {
        let idx = *self.index.get(key)?;
        Some(&self.node(idx).value)
    }

    /// Value for `key`, promoted to most recently used.
    pub(super) fn get(&mut self, key: &K) -> Option<&V> {
        let idx = *self.index.get(key)?;
        self.move_to_back(idx);
        Some(&self.node(idx).value)
    }

    /// Inserts or overwrites `key` as most recently used; returns whether
    /// the least recently used entry was evicted to make room.
    pub(super) fn insert(&mut self, key: K, value: V) -> bool {
        if let Some(&idx) = self.index.get(&key) {
            self.node_mut(idx).value = value;
            self.move_to_back(idx);
            false
        } else if self.nodes.len() < self.capacity {
            self.push_back(key, value);
            false
        } else {
            self.recycle_lru(key, value)
        }
    }

    pub(super) fn remove(&mut self, key: &K) -> Option<V> {
        let idx = *self.index.get(key)?;
        Some(self.remove_at(idx))
    }

    pub(super) fn clear(&mut self) {
        self.index.clear();
        self.nodes.clear();
        self.head = NIL;
        self.tail = NIL;
    }

    fn push_back(&mut self, key: K, value: V) {
        let idx = self.nodes.len();
        self.index.insert(key.clone(), idx);
        self.nodes.push(Node {
            key,
            value,
            prev: NIL,
            next: NIL,
        });
        self.link_back(idx);
    }

    /// Hands the LRU node to a new entry: no allocation, no relocation.
    /// Returns `false` only at capacity 0, where there is no node to take.
    fn recycle_lru(&mut self, key: K, value: V) -> bool {
        let idx = self.head;
        if idx == NIL {
            return false;
        }
        let node = self.node_mut(idx);
        let evicted = std::mem::replace(&mut node.key, key.clone());
        node.value = value;
        // Remove before insert: the index never exceeds the capacity it was
        // allocated for, so a full cache never rehashes.
        self.index.remove(&evicted);
        self.index.insert(key, idx);
        self.move_to_back(idx);
        true
    }

    fn move_to_back(&mut self, idx: usize) {
        if idx != self.tail {
            self.unlink(idx);
            self.link_back(idx);
        }
    }

    fn remove_at(&mut self, idx: usize) -> V {
        self.unlink(idx);
        let removed = self.nodes.swap_remove(idx);
        self.index.remove(&removed.key);
        if idx < self.nodes.len() {
            self.relocated_to(idx);
        }
        removed.value
    }

    /// Re-points the neighbours and the index entry of the node `swap_remove`
    /// just moved from the end of `nodes` into slot `idx`.
    fn relocated_to(&mut self, idx: usize) {
        let (prev, next) = self.links(idx);
        self.set_next(prev, idx);
        self.set_prev(next, idx);
        let key = &self.nodes[idx].key;
        if let Some(slot) = self.index.get_mut(key) {
            *slot = idx;
        }
    }

    fn unlink(&mut self, idx: usize) {
        let (prev, next) = self.links(idx);
        self.set_next(prev, next);
        self.set_prev(next, prev);
    }

    fn link_back(&mut self, idx: usize) {
        let old_tail = self.tail;
        let node = self.node_mut(idx);
        node.prev = old_tail;
        node.next = NIL;
        self.set_next(old_tail, idx);
        self.tail = idx;
    }

    fn links(&self, idx: usize) -> (usize, usize) {
        let node = self.node(idx);
        (node.prev, node.next)
    }

    /// `prev.next = next`, or the head when `prev` is the list start.
    fn set_next(&mut self, prev: usize, next: usize) {
        if prev == NIL {
            self.head = next;
        } else {
            self.node_mut(prev).next = next;
        }
    }

    /// `next.prev = prev`, or the tail when `next` is the list end.
    fn set_prev(&mut self, next: usize, prev: usize) {
        if next == NIL {
            self.tail = prev;
        } else {
            self.node_mut(next).prev = prev;
        }
    }

    fn node(&self, idx: usize) -> &Node<K, V> {
        self.touch();
        &self.nodes[idx]
    }

    fn node_mut(&mut self, idx: usize) -> &mut Node<K, V> {
        self.touch();
        &mut self.nodes[idx]
    }

    /// Counts a node access in test builds; compiles to nothing otherwise.
    #[inline]
    #[cfg_attr(not(test), allow(clippy::unused_self))]
    fn touch(&self) {
        #[cfg(test)]
        self.touched
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    #[cfg(test)]
    pub(super) fn touched(&self) -> u64 {
        self.touched.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Keys from LRU to MRU, after checking every structural invariant:
    /// back links mirror forward links and the index points at each node.
    #[cfg(test)]
    pub(super) fn checked_order(&self) -> Vec<K> {
        let mut order = Vec::with_capacity(self.nodes.len());
        let (mut prev, mut idx) = (NIL, self.head);
        while idx != NIL {
            let node = &self.nodes[idx];
            assert_eq!(node.prev, prev, "back link broken at slot {idx}");
            assert_eq!(
                self.index.get(&node.key),
                Some(&idx),
                "index stale at slot {idx}"
            );
            order.push(node.key.clone());
            (prev, idx) = (idx, node.next);
        }
        assert_eq!(self.tail, prev, "tail is not the last node");
        assert_eq!(order.len(), self.nodes.len(), "unreachable nodes");
        assert_eq!(self.index.len(), self.nodes.len(), "index size mismatch");
        order
    }
}
