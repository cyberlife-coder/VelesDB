//! Lock-Free LRU Cache with `DashMap` L1 (US-CORE-003-15)
//!
//! Two-tier cache architecture for maximum concurrent throughput:
//! - **L1**: `DashMap` (lock-free concurrent `HashMap`) for hot keys, with
//!   LRU eviction (see "L1 recency")
//! - **L2**: `LruCache` (with LRU eviction) for capacity management
//!
//! # Tier relationship
//!
//! Inserts write through to both tiers, and an L2 hit promotes the entry to
//! L1. An L1 hit stays lock-free and does not touch L2, so a key served from
//! L1 alone ages in L2 and may be evicted there while it is hot. When L1
//! evicts an entry, it demotes it to L2's MRU position (re-inserting it if
//! L2 had dropped it). Every access goes through L1, so keys leave L1 in
//! last-access order (up to the ties described under "L1 recency"), and
//! L2's order among the keys outside L1 is their last-access order: L2 acts
//! as L1's victim cache.
//!
//! # L1 recency
//!
//! `DashMap` keeps no order, so every L1 entry carries a last-access stamp
//! read from a shared logical clock. Only writes (insert, promotion)
//! advance it, by two: a write stamps its entry with the odd time in
//! between, strictly after every access before it and strictly before
//! every hit after it. An L1 hit only *reads* the clock, and stores into its
//! entry only when the entry's stamp is older: steady-state hits write no
//! shared memory beyond the hit counter.
//!
//! Over capacity, L1 evicts the entry with the oldest stamp. That is the
//! exact LRU entry, except among entries whose last access was a hit in the
//! same window between two writes, whose stamps tie: there, the victim is
//! any one of them. A per-hit clock would make the order exact, at the cost
//! of a contended write on every hit (measured at about half the concurrent
//! hit throughput). The removal is guarded on the victim's stamp, so an entry
//! hit in a later window while the victim was being chosen is not evicted:
//! the next pass picks the new oldest one instead. `peek_l1` does not
//! refresh the stamp.
//!
//! # Performance
//!
//! No recorded run times these operations; what each one touches:
//!
//! | Operation | L1 Hit | L1 Miss + L2 Hit |
//! |-----------|--------|------------------|
//! | `get()` | lock-free, stamp refresh if stale | L2, then promotion to L1 |
//! | `peek()` | L1 only | N/A |
//! | `insert()` | write-through | - |
//! | `remove()` | L1 + L2 | L1 + L2 (`LruCache::remove`, order-preserving) |
//!
//! When an insert or a promotion takes L1 over capacity, choosing the victim
//! scans L1, O(L1 capacity), and demoting it is one L2 insert (O(L2
//! capacity) under the L2 write lock), per evicted entry.

// Reason: Numeric casts in cache metrics are intentional:
// - All casts are for hit rate calculations and statistics
// - f64/u64 conversions for computing cache hit ratios
// - Values bounded by cache size and access patterns
// - Precision loss acceptable for cache metrics
#![allow(clippy::cast_precision_loss)]

use dashmap::{mapref::entry::Entry, DashMap};
use std::hash::Hash;
use std::sync::atomic::{AtomicU64, Ordering};

use super::LruCache;

/// Upper bound on L1 eviction passes per insert or promotion, so contention
/// cannot make a writer spin; a shortfall is counted in `l1_overflow`.
const MAX_L1_EVICTION_PASSES: usize = 10;

/// An L1 value and the logical time it was last inserted, promoted or hit.
struct L1Entry<V> {
    value: V,
    last_access: AtomicU64,
}

impl<V> L1Entry<V> {
    fn new(value: V, stamp: u64) -> Self {
        Self {
            value,
            last_access: AtomicU64::new(stamp),
        }
    }

    /// Records an access at `stamp`. Writes only when the entry is older, so
    /// repeated hits within one write window leave its cache line shared;
    /// `fetch_max` keeps the stamp monotonic under racing hits.
    fn touch(&self, stamp: u64) {
        if self.last_access.load(Ordering::Relaxed) < stamp {
            self.last_access.fetch_max(stamp, Ordering::Relaxed);
        }
    }

    fn last_access(&self) -> u64 {
        self.last_access.load(Ordering::Relaxed)
    }
}

/// Lock-free two-tier cache with `DashMap` L1 and `LruCache` L2.
///
/// Optimized for read-heavy workloads with hot keys.
pub struct LockFreeLruCache<K, V>
where
    K: Hash + Eq + Clone + Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
{
    /// L1: Lock-free concurrent cache for hot keys, evicted in LRU order.
    l1: DashMap<K, L1Entry<V>>,
    /// Logical clock that stamps L1 accesses; advanced by writes only.
    clock: AtomicU64,
    /// L2: LRU cache with eviction for capacity management.
    pub(crate) l2: LruCache<K, V>,
    /// Maximum L1 entries. Beyond it L1 demotes its least recently used
    /// entry to L2.
    l1_capacity: usize,
    /// L1 hit counter.
    l1_hits: AtomicU64,
    /// L2 hit counter (L1 miss, L2 hit).
    l2_hits: AtomicU64,
    /// Total miss counter.
    misses: AtomicU64,
    /// L1 eviction overflow counter (P2 Audit 2026-01-29).
    /// Counts times when L1 exceeded capacity after max eviction attempts.
    l1_overflow: AtomicU64,
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
    /// L1 overflow count (times L1 exceeded capacity after max eviction attempts).
    pub l1_overflow: u64,
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
            clock: AtomicU64::new(0),
            l2: LruCache::new(l2_capacity),
            l1_capacity,
            l1_hits: AtomicU64::new(0),
            l2_hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
            l1_overflow: AtomicU64::new(0),
        }
    }

    /// Get a value, checking L1 first then L2.
    ///
    /// If found in L2, promotes to L1 for faster subsequent access.
    #[must_use]
    pub fn get(&self, key: &K) -> Option<V> {
        // Fast path: L1 lookup (lock-free)
        if let Some(entry) = self.l1.get(key) {
            entry.touch(self.now());
            self.l1_hits.fetch_add(1, Ordering::Relaxed);
            return Some(entry.value.clone());
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
    /// Returns None if not in L1, even if present in L2. Does not refresh
    /// the entry's recency.
    #[must_use]
    pub fn peek_l1(&self, key: &K) -> Option<V> {
        self.l1.get(key).map(|entry| entry.value.clone())
    }

    /// Peek at L2 only (no L1 check, no promotion).
    #[must_use]
    pub fn peek_l2(&self, key: &K) -> Option<V> {
        self.l2.peek(key)
    }

    /// Insert a key-value pair (write-through to L1 and L2).
    pub fn insert(&self, key: K, value: V) {
        // Write to L1
        self.l1.insert(
            key.clone(),
            L1Entry::new(value.clone(), self.advance_clock()),
        );

        // Evict from L1 if over capacity
        self.maybe_evict_l1();

        // Write to L2 (backing store)
        self.l2.insert(key, value);
    }

    /// Remove a key from both L1 and L2.
    ///
    /// L1 first: a demotion of this key holds its L1 shard lock until its
    /// L2 copy is written, so the L2 removal that follows deletes that copy.
    pub fn remove(&self, key: &K) {
        self.l1.remove(key);
        self.l2.remove(key);
    }

    /// Clear both L1 and L2.
    ///
    /// L1 first, for the same reason as [`remove`](Self::remove).
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
            l1_overflow: self.l1_overflow.load(Ordering::Relaxed),
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

    /// Current time of the L1 logical clock, for a hit.
    fn now(&self) -> u64 {
        self.clock.load(Ordering::Relaxed)
    }

    /// Advances the L1 logical clock by two for a write and returns the odd
    /// time in between: later than every earlier access, earlier than every
    /// hit that reads the clock after it.
    fn advance_clock(&self) -> u64 {
        self.clock.fetch_add(2, Ordering::Relaxed) + 1
    }

    /// Promote a key from L2 to L1 as its most recently used entry.
    fn promote_to_l1(&self, key: K, value: V) {
        self.l1
            .insert(key, L1Entry::new(value, self.advance_clock()));
        self.maybe_evict_l1();
    }

    /// Demote least recently used L1 entries to L2 while L1 is over capacity.
    ///
    /// Bounded to [`MAX_L1_EVICTION_PASSES`] so contention cannot make the
    /// caller spin.
    fn maybe_evict_l1(&self) {
        for _ in 0..MAX_L1_EVICTION_PASSES {
            if self.l1.len() <= self.l1_capacity {
                break;
            }
            let Some((victim, stamp)) = self.least_recently_used_l1() else {
                break;
            };
            self.demote_to_l2(victim, stamp);
        }

        // P2 Audit 2026-01-29: Track overflow when L1 still exceeds capacity after max attempts
        if self.l1.len() > self.l1_capacity {
            self.l1_overflow.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Moves `victim` from L1 to L2's MRU position, unless an access
    /// refreshed its stamp since it was chosen: the entry then stays, and the
    /// next pass picks the new oldest one.
    ///
    /// Both steps run under the victim's L1 shard write lock, which
    /// `remove` and `clear` take before touching L2: they either find the
    /// victim still in L1, or run after the demotion and delete its L2 copy.
    /// A removed key is never re-inserted into L2. Lock order: L1 shard,
    /// then `LruCache.inner`.
    fn demote_to_l2(&self, victim: K, stamp: u64) {
        if let Entry::Occupied(slot) = self.l1.entry(victim) {
            if slot.get().last_access() == stamp {
                self.l2.insert(slot.key().clone(), slot.get().value.clone());
                slot.remove();
            }
        }
    }

    /// The L1 key with the oldest access stamp, and that stamp.
    ///
    /// Clones a key only when it beats the current minimum, and takes no
    /// lock beyond the shard guards of the iteration, released on return.
    fn least_recently_used_l1(&self) -> Option<(K, u64)> {
        let mut oldest: Option<(K, u64)> = None;
        for entry in &self.l1 {
            let stamp = entry.last_access();
            if oldest.as_ref().is_none_or(|&(_, min)| stamp < min) {
                oldest = Some((entry.key().clone(), stamp));
            }
        }
        oldest
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
