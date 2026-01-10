//! LRU Cache implementation for VelesDB.
//!
//! Thread-safe LRU cache with statistics tracking.
//! Based on arXiv:2310.11703v2 recommendations.

#![allow(clippy::cast_precision_loss)] // Precision loss acceptable for hit rate calculation

use parking_lot::RwLock;
use rustc_hash::FxHashMap;
use std::collections::VecDeque;
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
pub struct LruCache<K, V>
where
    K: Hash + Eq + Clone,
    V: Clone,
{
    /// Maximum capacity.
    capacity: usize,
    /// Internal data protected by `RwLock`.
    inner: RwLock<LruInner<K, V>>,
    /// Statistics (atomic for lock-free reads).
    hits: AtomicU64,
    misses: AtomicU64,
    evictions: AtomicU64,
}

struct LruInner<K, V> {
    /// Key -> Value map.
    map: FxHashMap<K, V>,
    /// Order queue (front = LRU, back = MRU).
    order: VecDeque<K>,
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
            inner: RwLock::new(LruInner {
                map: FxHashMap::default(),
                order: VecDeque::with_capacity(capacity),
            }),
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
        self.inner.read().map.len()
    }

    /// Check if the cache is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.read().map.is_empty()
    }

    /// Insert a key-value pair, evicting LRU entry if at capacity.
    pub fn insert(&self, key: K, value: V) {
        let mut inner = self.inner.write();

        // Check if key already exists
        if inner.map.contains_key(&key) {
            // Update value and move to back (MRU)
            inner.map.insert(key.clone(), value);
            self.move_to_back(&mut inner.order, &key);
            return;
        }

        // Evict if at capacity
        if inner.map.len() >= self.capacity {
            if let Some(evicted_key) = inner.order.pop_front() {
                inner.map.remove(&evicted_key);
                self.evictions.fetch_add(1, Ordering::Relaxed);
            }
        }

        // Insert new entry
        inner.map.insert(key.clone(), value);
        inner.order.push_back(key);
    }

    /// Get a value by key, updating recency.
    #[must_use]
    pub fn get(&self, key: &K) -> Option<V> {
        let mut inner = self.inner.write();

        if let Some(value) = inner.map.get(key).cloned() {
            self.hits.fetch_add(1, Ordering::Relaxed);
            self.move_to_back(&mut inner.order, key);
            Some(value)
        } else {
            self.misses.fetch_add(1, Ordering::Relaxed);
            None
        }
    }

    /// Remove a key from the cache.
    pub fn remove(&self, key: &K) {
        let mut inner = self.inner.write();

        if inner.map.remove(key).is_some() {
            inner.order.retain(|k| k != key);
        }
    }

    /// Clear all entries.
    pub fn clear(&self) {
        let mut inner = self.inner.write();
        inner.map.clear();
        inner.order.clear();
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

    /// Move a key to the back of the order queue (most recently used).
    fn move_to_back(&self, order: &mut VecDeque<K>, key: &K) {
        // Remove from current position
        order.retain(|k| k != key);
        // Add to back
        order.push_back(key.clone());
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
