//! LRU Cache implementation for `VelesDB`.
//!
//! Thread-safe LRU cache: an [`LruList`] (hash map over a doubly linked
//! slab, front = LRU, back = MRU) behind a `RwLock`, plus hit, miss and
//! eviction counters. Based on arXiv:2310.11703v2 recommendations.
//!
//! # Performance (US-CORE-003-14)
//!
//! Every operation is O(1): one hash lookup and a constant number of link
//! updates.
//!
//! | Operation | Lock | Notes |
//! |-----------|------|-------|
//! | insert | write | Moves an existing key to MRU; evicts the LRU entry when full |
//! | get | write | Moves the entry to MRU |
//! | peek | read | No recency update |
//! | remove | write | Keeps the order of the remaining entries |

#![allow(clippy::cast_precision_loss)] // Precision loss acceptable for hit rate calculation

use super::lru_list::LruList;
use parking_lot::RwLock;
use std::hash::Hash;
use std::sync::atomic::{AtomicU64, Ordering};

/// Cache statistics for monitoring.
#[derive(Debug, Clone, Default)]
pub struct CacheStats {
    /// Number of cache hits.
    pub hits: u64,
    /// Number of cache misses.
    pub misses: u64,
    /// Number of entries evicted to make room for a new key.
    pub evictions: u64,
}

impl CacheStats {
    /// Calculate hit rate (0.0 to 1.0).
    #[must_use]
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            self.hits as f64 / total as f64
        }
    }
}

/// Thread-safe LRU cache.
///
/// See the module docs for the structure and per-operation costs.
pub struct LruCache<K, V>
where
    K: Hash + Eq + Clone,
    V: Clone,
{
    /// Maximum capacity.
    capacity: usize,
    /// Entries in recency order (front = LRU, back = MRU).
    inner: RwLock<LruList<K, V>>,
    /// Statistics (atomic for lock-free reads).
    hits: AtomicU64,
    misses: AtomicU64,
    evictions: AtomicU64,
}

impl<K, V> LruCache<K, V>
where
    K: Hash + Eq + Clone,
    V: Clone,
{
    /// Create a new LRU cache with the given capacity.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            inner: RwLock::new(LruList::new(capacity)),
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
            evictions: AtomicU64::new(0),
        }
    }

    /// Get the capacity of the cache.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Get the current number of entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.read().len()
    }

    /// Check if the cache is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.read().is_empty()
    }

    /// Insert a key-value pair as the most recently used entry, evicting the
    /// least recently used one if a new key meets a full cache.
    ///
    /// A zero-capacity cache stores nothing, and returns without locking.
    pub fn insert(&self, key: K, value: V) {
        if self.capacity == 0 {
            return;
        }
        let evicted = self.inner.write().insert(key, value);
        if evicted.is_some() {
            self.evictions.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Get a value by key; the entry becomes the most recently used.
    ///
    /// One write lock covers the lookup and the move to MRU (F-18).
    #[must_use]
    pub fn get(&self, key: &K) -> Option<V> {
        let value = self.inner.write().get(key).cloned();
        let counter = if value.is_some() {
            &self.hits
        } else {
            &self.misses
        };
        counter.fetch_add(1, Ordering::Relaxed);
        value
    }

    /// Get a value without updating recency, under a read lock.
    #[must_use]
    pub fn peek(&self, key: &K) -> Option<V> {
        self.inner.read().peek(key).cloned()
    }

    /// Remove a key, keeping the order of the remaining entries.
    pub fn remove(&self, key: &K) {
        self.inner.write().remove(key);
    }

    /// Clear all entries.
    pub fn clear(&self) {
        let mut inner = self.inner.write();
        inner.clear();
    }

    /// Get cache statistics.
    #[must_use]
    pub fn stats(&self) -> CacheStats {
        CacheStats {
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
            evictions: self.evictions.load(Ordering::Relaxed),
        }
    }

    /// List nodes touched while `op` runs: the structural complexity probe.
    #[cfg(test)]
    pub(super) fn nodes_touched_by(&self, op: impl FnOnce(&Self)) -> u64 {
        let before = self.inner.read().nodes_touched();
        op(self);
        self.inner.read().nodes_touched() - before
    }
}

impl<K, V> Default for LruCache<K, V>
where
    K: Hash + Eq + Clone,
    V: Clone,
{
    fn default() -> Self {
        Self::new(10_000) // Default 10K entries
    }
}
