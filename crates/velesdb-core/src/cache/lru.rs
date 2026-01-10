//! LRU Cache implementation for VelesDB.
//!
//! Thread-safe LRU cache with O(1) operations using IndexMap.
//! Based on arXiv:2310.11703v2 recommendations.
//!
//! # Performance (US-CORE-003-14)
//!
//! | Operation | Complexity | Notes |
//! |-----------|------------|-------|
//! | insert | O(1) | Amortized |
//! | get | O(1) | With recency update |
//! | remove | O(1) | swap_remove |
//! | eviction | O(1) | shift_remove from front |

#![allow(clippy::cast_precision_loss)] // Precision loss acceptable for hit rate calculation

use indexmap::IndexMap;
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
/// Uses `IndexMap` internally which preserves insertion order
/// and provides O(1) access, making move-to-back operations efficient.
pub struct LruCache<K, V>
where
    K: Hash + Eq + Clone,
    V: Clone,
{
    /// Maximum capacity.
    capacity: usize,
    /// Internal data protected by `RwLock`.
    /// IndexMap preserves insertion order (front = LRU, back = MRU).
    inner: RwLock<IndexMap<K, V>>,
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
            inner: RwLock::new(IndexMap::with_capacity(capacity)),
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

    /// Insert a key-value pair, evicting LRU entry if at capacity.
    ///
    /// O(1) amortized complexity.
    pub fn insert(&self, key: K, value: V) {
        let mut inner = self.inner.write();

        // Check if key already exists - if so, remove and re-insert to move to back
        if inner.shift_remove(&key).is_some() {
            // Key existed, just re-insert at back
            inner.insert(key, value);
            return;
        }

        // Evict LRU (front) if at capacity
        if inner.len() >= self.capacity {
            // shift_remove(0) removes the first element (LRU)
            if inner.shift_remove_index(0).is_some() {
                self.evictions.fetch_add(1, Ordering::Relaxed);
            }
        }

        // Insert new entry at back (MRU)
        inner.insert(key, value);
    }

    /// Get a value by key, updating recency.
    ///
    /// O(1) complexity with read lock for lookup, write lock for recency update.
    #[must_use]
    pub fn get(&self, key: &K) -> Option<V> {
        // Fast path: read lock to check existence and get value
        let value = {
            let inner = self.inner.read();
            inner.get(key).cloned()
        };

        match value {
            Some(v) => {
                self.hits.fetch_add(1, Ordering::Relaxed);
                // Update recency: remove and re-insert at back
                self.move_to_back(key, &v);
                Some(v)
            }
            None => {
                self.misses.fetch_add(1, Ordering::Relaxed);
                None
            }
        }
    }

    /// Get a value without updating recency (peek).
    ///
    /// O(1) complexity with only read lock.
    #[must_use]
    pub fn peek(&self, key: &K) -> Option<V> {
        let inner = self.inner.read();
        inner.get(key).cloned()
    }

    /// Remove a key from the cache.
    ///
    /// O(1) complexity using swap_remove (doesn't preserve order of other elements).
    pub fn remove(&self, key: &K) {
        let mut inner = self.inner.write();
        inner.swap_remove(key);
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

    /// Move a key to the back (most recently used).
    /// O(1) amortized using shift_remove + insert.
    fn move_to_back(&self, key: &K, value: &V) {
        let mut inner = self.inner.write();
        // Remove from current position
        inner.shift_remove(key);
        // Re-insert at back
        inner.insert(key.clone(), value.clone());
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
