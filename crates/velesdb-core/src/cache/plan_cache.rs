//! Compiled query plan cache types for `VelesDB` (CACHE-01).
//!
//! Provides `PlanKey` (deterministic cache key), `CompiledPlan` (cached execution plan),
//! `PlanCacheMetrics` (hit/miss counters), and `CompiledPlanCache` (thin wrapper around
//! `LockFreeLruCache`).

use std::fmt;
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
///
/// # Correctness invariant (CACHE-01)
///
/// Cache correctness depends on `query_hash` capturing **all** collection
/// names referenced by the query (base table + JOIN targets). If two
/// structurally different queries happened to hash to the same value the cache
/// would serve a stale plan. This is prevented by using the full canonical
/// serialization of the `Query` AST — not just the collection name — as the
/// hash input (see `Database::build_plan_key`). The `collection_generations`
/// vector is then ordered by collection name so that the same set of
/// collections always produces the same key regardless of JOIN ordering.
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
    ///
    /// Currently stale-key detection in `build_plan_key` handles invalidation:
    /// when a collection's `write_generation` changes the key no longer matches
    /// anything in the cache so a fresh plan is compiled on the next call.
    ///
    /// Future work (CACHE-01): use `referenced_collections` for targeted
    /// invalidation — evict only plans that touch a mutated collection rather
    /// than relying on stale-key detection. This requires an inverted index
    /// from collection name to `PlanKey` and would reduce spurious misses in
    /// multi-collection workloads.
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
    #[allow(clippy::cast_precision_loss)]
    pub fn hit_rate(&self) -> f64 {
        let h = self.hits();
        let m = self.misses();
        let total = h + m;
        if total == 0 {
            0.0
        } else {
            // Precision loss is acceptable: hit rate is a diagnostic metric,
            // not a value used in any computation where exactness matters.
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

impl fmt::Debug for CompiledPlanCache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let stats = self.cache.stats();
        f.debug_struct("CompiledPlanCache")
            .field("l1_size", &stats.l1_size)
            .field("l2_size", &stats.l2_size)
            .field("hits", &self.metrics.hits())
            .field("misses", &self.metrics.misses())
            .finish()
    }
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

    /// Returns `true` if a plan for `key` exists in the cache.
    ///
    /// Unlike [`get`](Self::get), this method does **not** record a hit or miss
    /// in the metrics counters and does **not** increment `reuse_count`. It is
    /// intended for existence checks (e.g. deciding whether to insert a newly
    /// compiled plan) where polluting the metrics would distort hit-rate
    /// calculations.
    #[must_use]
    pub fn contains(&self, key: &PlanKey) -> bool {
        // Check L1 first (lock-free DashMap), then L2 (LRU behind a mutex).
        // Using peek_l1 / peek_l2 avoids the LRU promotion that `get` would
        // trigger, keeping the hot-path ordering stable.
        self.cache.peek_l1(key).is_some() || self.cache.peek_l2(key).is_some()
    }

    /// Looks up a compiled plan by key, recording hit/miss.
    #[must_use]
    pub fn get(&self, key: &PlanKey) -> Option<Arc<CompiledPlan>> {
        if let Some(plan) = self.cache.get(key) {
            self.metrics.record_hit();
            plan.reuse_count.fetch_add(1, Ordering::Relaxed);
            Some(plan)
        } else {
            self.metrics.record_miss();
            None
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
