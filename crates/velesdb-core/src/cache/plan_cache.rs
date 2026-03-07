//! Compiled query plan cache types for `VelesDB` (CACHE-01).
//!
//! Provides `PlanKey` (deterministic cache key), `CompiledPlan` (cached execution plan),
//! `PlanCacheMetrics` (hit/miss counters), and `CompiledPlanCache` (thin wrapper around
//! `LockFreeLruCache`).

// SAFETY: Numeric casts in cache metrics are intentional:
// - f64/u64 conversions for computing cache hit ratios
// - Values bounded by cache size and access patterns
// - Precision loss acceptable for cache metrics
#![allow(clippy::cast_precision_loss)]

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use smallvec::SmallVec;

use super::LockFreeLruCache;
use crate::velesql::QueryPlan;

/// Deterministic cache key for compiled query plans.
///
/// Two keys are equal iff the query text is identical AND the database state
/// (schema version + per-collection write generations) has not changed.
///
/// `collection_generations` must be sorted by collection name before
/// insertion for deterministic hashing.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PlanKey {
    /// `FxHash` of canonical query text.
    pub query_hash: u64,
    /// Monotonic counter incremented on every DDL operation.
    pub schema_version: u64,
    /// Per-collection write generation, sorted by collection name.
    pub collection_generations: SmallVec<[u64; 4]>,
}

/// A compiled (cached) query execution plan.
///
/// Stored behind `Arc` in the cache; the cache value type is
/// `Arc<CompiledPlan>` so `Clone` is not required on this struct.
#[derive(Debug)]
pub struct CompiledPlan {
    /// The query plan produced by the planner.
    pub plan: QueryPlan,
    /// Collections referenced by this plan (for invalidation checks).
    pub referenced_collections: Vec<String>,
    /// When this plan was compiled.
    pub compiled_at: std::time::Instant,
    /// How many times this cached plan has been reused.
    pub reuse_count: AtomicU64,
}

/// Global cache hit/miss counters.
#[derive(Debug, Default)]
pub struct PlanCacheMetrics {
    /// Total cache hits.
    pub hits: AtomicU64,
    /// Total cache misses.
    pub misses: AtomicU64,
}

impl PlanCacheMetrics {
    /// Records a cache hit.
    pub fn record_hit(&self) {
        self.hits.fetch_add(1, Ordering::Relaxed);
    }

    /// Records a cache miss.
    pub fn record_miss(&self) {
        self.misses.fetch_add(1, Ordering::Relaxed);
    }

    /// Returns total hits.
    #[must_use]
    pub fn hits(&self) -> u64 {
        self.hits.load(Ordering::Relaxed)
    }

    /// Returns total misses.
    #[must_use]
    pub fn misses(&self) -> u64 {
        self.misses.load(Ordering::Relaxed)
    }

    /// Returns the hit rate as a ratio in `[0.0, 1.0]`.
    #[must_use]
    pub fn hit_rate(&self) -> f64 {
        let h = self.hits();
        let m = self.misses();
        let total = h + m;
        if total == 0 {
            0.0
        } else {
            h as f64 / total as f64
        }
    }
}

/// Thin wrapper around `LockFreeLruCache` for compiled query plans.
///
/// Tracks hit/miss metrics and delegates storage to the lock-free two-tier cache.
pub struct CompiledPlanCache {
    cache: LockFreeLruCache<PlanKey, Arc<CompiledPlan>>,
    metrics: PlanCacheMetrics,
}

impl CompiledPlanCache {
    /// Creates a new compiled plan cache.
    ///
    /// # Arguments
    ///
    /// * `l1_capacity` - Maximum entries in L1 (hot cache)
    /// * `l2_capacity` - Maximum entries in L2 (LRU backing store)
    #[must_use]
    pub fn new(l1_capacity: usize, l2_capacity: usize) -> Self {
        Self {
            cache: LockFreeLruCache::new(l1_capacity, l2_capacity),
            metrics: PlanCacheMetrics::default(),
        }
    }

    /// Looks up a compiled plan by key, recording hit/miss.
    #[must_use]
    pub fn get(&self, key: &PlanKey) -> Option<Arc<CompiledPlan>> {
        match self.cache.get(key) {
            Some(plan) => {
                self.metrics.record_hit();
                plan.reuse_count.fetch_add(1, Ordering::Relaxed);
                Some(plan)
            }
            None => {
                self.metrics.record_miss();
                None
            }
        }
    }

    /// Inserts a compiled plan into the cache.
    pub fn insert(&self, key: PlanKey, plan: Arc<CompiledPlan>) {
        self.cache.insert(key, plan);
    }

    /// Returns the underlying cache statistics.
    #[must_use]
    pub fn stats(&self) -> super::LockFreeCacheStats {
        self.cache.stats()
    }

    /// Returns a reference to the plan cache metrics.
    #[must_use]
    pub fn metrics(&self) -> &PlanCacheMetrics {
        &self.metrics
    }
}
