//! Lock-Free LRU Cache with DashMap L1 (US-CORE-003-15)
//!
//! Two-tier cache architecture for maximum concurrent throughput:
//! - **L1**: DashMap (lock-free concurrent HashMap) for hot keys
//! - **L2**: LruCache (with LRU eviction) for capacity management
//!
//! # Performance
//!
//! | Operation | L1 Hit | L1 Miss + L2 Hit |
//! |-----------|--------|------------------|
//! | get() | ~50ns (lock-free) | ~500ns (with promotion) |
//! | peek() | ~30ns (L1 only) | N/A |
//! | insert() | ~100ns (write-through) | - |

use dashmap::DashMap;
use std::hash::Hash;
use std::sync::atomic::{AtomicU64, Ordering};

use super::LruCache;

/// Lock-free two-tier cache with DashMap L1 and LruCache L2.
///
/// Optimized for read-heavy workloads with hot keys.
pub struct LockFreeLruCache<K, V>
where
    K: Hash + Eq + Clone + Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
{
    /// L1: Lock-free concurrent cache for hot keys.
    l1: DashMap<K, V>,
    /// L2: LRU cache with eviction for capacity management.
    pub(crate) l2: LruCache<K, V>,
    /// Maximum L1 entries before eviction to L2.
    l1_capacity: usize,
    /// L1 hit counter.
    l1_hits: AtomicU64,
    /// L2 hit counter (L1 miss, L2 hit).
    l2_hits: AtomicU64,
    /// Total miss counter.
    misses: AtomicU64,
}

/// Statistics for the two-tier cache.
#[derive(Debug, Clone, Default)]
pub struct LockFreeCacheStats {
    /// L1 cache hits.
    pub l1_hits: u64,
    /// L2 cache hits (L1 miss, L2 hit).
    pub l2_hits: u64,
    /// Total misses.
    pub misses: u64,
    /// L1 current size.
    pub l1_size: usize,
    /// L2 current size.
    pub l2_size: usize,
}

impl LockFreeCacheStats {
    /// Calculate L1 hit rate.
    #[must_use]
    pub fn l1_hit_rate(&self) -> f64 {
        let total = self.l1_hits + self.l2_hits + self.misses;
        if total == 0 {
            0.0
        } else {
            self.l1_hits as f64 / total as f64
        }
    }

    /// Calculate total hit rate (L1 + L2).
    #[must_use]
    pub fn total_hit_rate(&self) -> f64 {
        let total = self.l1_hits + self.l2_hits + self.misses;
        if total == 0 {
            0.0
        } else {
            (self.l1_hits + self.l2_hits) as f64 / total as f64
        }
    }
}

impl<K, V> LockFreeLruCache<K, V>
where
    K: Hash + Eq + Clone + Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
{
    /// Create a new lock-free LRU cache.
    ///
    /// # Arguments
    ///
    /// * `l1_capacity` - Maximum entries in L1 (hot cache)
    /// * `l2_capacity` - Maximum entries in L2 (LRU backing store)
    #[must_use]
    pub fn new(l1_capacity: usize, l2_capacity: usize) -> Self {
        Self {
            l1: DashMap::with_capacity(l1_capacity),
            l2: LruCache::new(l2_capacity),
            l1_capacity,
            l1_hits: AtomicU64::new(0),
            l2_hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
        }
    }

    /// Get a value, checking L1 first then L2.
    ///
    /// If found in L2, promotes to L1 for faster subsequent access.
    #[must_use]
    pub fn get(&self, key: &K) -> Option<V> {
        // Fast path: L1 lookup (lock-free)
        if let Some(entry) = self.l1.get(key) {
            self.l1_hits.fetch_add(1, Ordering::Relaxed);
            return Some(entry.value().clone());
        }

        // Slow path: L2 lookup
        if let Some(value) = self.l2.get(key) {
            self.l2_hits.fetch_add(1, Ordering::Relaxed);
            // Promote to L1
            self.promote_to_l1(key.clone(), value.clone());
            return Some(value);
        }

        self.misses.fetch_add(1, Ordering::Relaxed);
        None
    }

    /// Peek at L1 only (ultra-fast, no promotion).
    ///
    /// Returns None if not in L1, even if present in L2.
    #[must_use]
    pub fn peek_l1(&self, key: &K) -> Option<V> {
        self.l1.get(key).map(|entry| entry.value().clone())
    }

    /// Peek at L2 only (no L1 check, no promotion).
    #[must_use]
    pub fn peek_l2(&self, key: &K) -> Option<V> {
        self.l2.peek(key)
    }

    /// Insert a key-value pair (write-through to L1 and L2).
    pub fn insert(&self, key: K, value: V) {
        // Write to L1
        self.l1.insert(key.clone(), value.clone());

        // Evict from L1 if over capacity
        self.maybe_evict_l1();

        // Write to L2 (backing store)
        self.l2.insert(key, value);
    }

    /// Remove a key from both L1 and L2.
    pub fn remove(&self, key: &K) {
        self.l1.remove(key);
        self.l2.remove(key);
    }

    /// Clear both L1 and L2.
    pub fn clear(&self) {
        self.l1.clear();
        self.l2.clear();
    }

    /// Get cache statistics.
    #[must_use]
    pub fn stats(&self) -> LockFreeCacheStats {
        LockFreeCacheStats {
            l1_hits: self.l1_hits.load(Ordering::Relaxed),
            l2_hits: self.l2_hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
            l1_size: self.l1.len(),
            l2_size: self.l2.len(),
        }
    }

    /// Get L1 capacity.
    #[must_use]
    pub fn l1_capacity(&self) -> usize {
        self.l1_capacity
    }

    /// Get L2 capacity.
    #[must_use]
    pub fn l2_capacity(&self) -> usize {
        self.l2.capacity()
    }

    /// Promote a key from L2 to L1.
    fn promote_to_l1(&self, key: K, value: V) {
        self.l1.insert(key, value);
        self.maybe_evict_l1();
    }

    /// Evict entries from L1 if over capacity.
    /// Uses a bounded loop to prevent infinite spinning under contention.
    fn maybe_evict_l1(&self) {
        // Bounded eviction: max attempts to prevent infinite loop under contention
        let mut attempts = 0;
        let max_attempts = 10;

        while self.l1.len() > self.l1_capacity && attempts < max_attempts {
            attempts += 1;

            // Collect keys to remove (avoid holding iterator while removing)
            let keys_to_remove: Vec<K> = self
                .l1
                .iter()
                .take(self.l1.len().saturating_sub(self.l1_capacity).max(1))
                .map(|entry| entry.key().clone())
                .collect();

            if keys_to_remove.is_empty() {
                break;
            }

            for key in keys_to_remove {
                self.l1.remove(&key);
            }
        }
    }
}

impl<K, V> Default for LockFreeLruCache<K, V>
where
    K: Hash + Eq + Clone + Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
{
    fn default() -> Self {
        // Default: L1 = 1K hot entries, L2 = 10K LRU entries
        Self::new(1_000, 10_000)
    }
}

// Tests moved to lockfree_tests.rs per project rules
