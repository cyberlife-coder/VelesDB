//! LRU Cache implementation for `VelesDB`.
//!
//! Thread-safe LRU cache with O(1) operations over [`LruList`].
//! Based on arXiv:2310.11703v2 recommendations.
//!
//! # Performance (US-CORE-003-14)
//!
//! | Operation | Complexity | Notes |
//! |-----------|------------|-------|
//! | insert | O(1) | Amortized: nodes allocated once, hash index rehashes tombstones |
//! | get | O(1) | With recency update (list relink) |
//! | remove | O(1) | Recency order of the others preserved |
//! | eviction | O(1) | LRU node reused in place by the incoming entry |
//!
//! Guarded by `lru_optimization_tests` (node-touch count and timing minima).

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
    /// Number of evictions.
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

/// Thread-safe LRU cache with O(1) operations.
///
/// Recency is kept by an index-linked list plus a key index ([`LruList`]),
/// so promote, evict and remove never shift other entries.
pub struct LruCache<K, V>
where
    K: Hash + Eq + Clone,
    V: Clone,
{
    /// Internal data protected by `RwLock`.
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
            inner: RwLock::new(LruList::new(capacity)),
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
            evictions: AtomicU64::new(0),
        }
    }

    /// Get the capacity of the cache.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.inner.read().capacity()
    }

    /// Get the current number of entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.read().len()
    }

    /// Check if the cache is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.read().len() == 0
    }

    /// Insert a key-value pair, evicting LRU entry if at capacity.
    ///
    /// O(1) amortized: nodes are allocated up front and the evicted node is
    /// reused; only the hash index occasionally rehashes (see `LruList`).
    pub fn insert(&self, key: K, value: V) {
        let evicted = self.inner.write().insert(key, value);
        if evicted {
            self.evictions.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Get a value by key, updating recency.
    ///
    /// F-18: Uses a single write lock for lookup + move-to-back in one operation,
    /// eliminating the previous read-lock → clone → write-lock → re-clone pattern
    /// (2 locks + 2 clones → 1 lock + 1 clone).
    #[must_use]
    pub fn get(&self, key: &K) -> Option<V> {
        let mut inner = self.inner.write();
        if let Some(value) = inner.get(key) {
            let cloned = value.clone();
            drop(inner);
            self.hits.fetch_add(1, Ordering::Relaxed);
            Some(cloned)
        } else {
            drop(inner);
            self.misses.fetch_add(1, Ordering::Relaxed);
            None
        }
    }

    /// Get a value without updating recency (peek).
    ///
    /// O(1) complexity with only read lock.
    #[must_use]
    pub fn peek(&self, key: &K) -> Option<V> {
        let inner = self.inner.read();
        inner.peek(key).cloned()
    }

    /// Remove a key from the cache.
    ///
    /// O(1) complexity; the recency order of the other entries is preserved.
    pub fn remove(&self, key: &K) {
        let mut inner = self.inner.write();
        inner.remove(key);
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
        let before = self.inner.read().touched();
        op(self);
        self.inner.read().touched() - before
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
