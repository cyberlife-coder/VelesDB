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
/// Two keys are equal iff the **canonical query text is byte-for-byte
/// identical** AND the database state (schema version + per-collection write /
/// analyze generations) has not changed.
///
/// `collection_generations` must be sorted by collection name before
/// insertion for deterministic hashing.
///
/// # Correctness invariant (CACHE-01, issue #902)
///
/// `query_hash` is a 64-bit `FxHash` of the canonical query text. `FxHash` is a
/// fast, **non-cryptographic** hash: distinct queries can be engineered to
/// collide on the same 64-bit value. If equality were decided on `query_hash`
/// alone, a colliding query would be treated as equal to an unrelated cached
/// query and the cache could return **another query's compiled plan** (wrong
/// results).
///
/// To make this impossible, `PlanKey` stores the canonical `query_text` and the
/// hand-written [`PartialEq`]/[`Eq`] impls compare the full text (not the
/// hash). `query_hash` is retained purely as a [`Hash`] accelerator so that the
/// `DashMap`/`LruCache` buckets stay cheap to compute; the `Hash` impl
/// deliberately feeds only `query_hash` (plus the generation fields) into the
/// hasher rather than re-hashing the whole string on every lookup. This mirrors
/// the safe pattern used by the `VelesQL` parse cache (`velesql/cache.rs`),
/// which re-checks query-text equality on every lookup.
///
/// `Hash`/`Eq` consistency: equal keys have identical `query_text`, and
/// `query_hash` is a pure function of `query_text`, so equal keys always hash
/// identically — the `Hash`/`Eq` contract holds.
#[derive(Clone, Debug)]
pub struct PlanKey {
    /// Canonical serialization of the query AST (see `Database::build_plan_key`).
    ///
    /// This is the authoritative query-identity field: equality is decided on
    /// this string, never on `query_hash` alone. Stored behind `Arc<str>` so
    /// cloning a `PlanKey` (done on every cache insert / promotion) is a cheap
    /// refcount bump rather than a full string copy.
    pub query_text: Arc<str>,
    /// `FxHash` of `query_text`. Hash accelerator only — **not** an identity
    /// field. Collisions are resolved by the `query_text` comparison in `Eq`.
    pub query_hash: u64,
    /// Monotonic counter incremented on every DDL operation.
    pub schema_version: u64,
    /// Per-collection write generation, sorted by collection name.
    pub collection_generations: SmallVec<[u64; 4]>,
    /// Per-collection analyze generation, sorted by collection name (issue #608).
    ///
    /// Parallel to `collection_generations` but tracks `ANALYZE` invocations
    /// rather than data mutations. Including this in the cache key ensures
    /// that running `ANALYZE` alone — with no intermediate write to bump
    /// `write_generation` — still invalidates plans whose cost estimates
    /// were derived from pre-analyze heuristics.
    pub analyze_generations: SmallVec<[u64; 4]>,
}

impl PartialEq for PlanKey {
    /// Equality compares the **canonical query text** (collision-safe), never
    /// `query_hash` alone (issue #902). The generation fields gate cache
    /// invalidation.
    fn eq(&self, other: &Self) -> bool {
        self.schema_version == other.schema_version
            && self.collection_generations == other.collection_generations
            && self.analyze_generations == other.analyze_generations
            && self.query_text == other.query_text
    }
}

impl Eq for PlanKey {}

impl std::hash::Hash for PlanKey {
    /// Feeds only the cheap `query_hash` accelerator (plus generation fields)
    /// into the hasher — never the full `query_text` — so lookups stay fast.
    /// Equal keys hash identically because `query_hash` is a pure function of
    /// `query_text`.
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.query_hash.hash(state);
        self.schema_version.hash(state);
        self.collection_generations.hash(state);
        self.analyze_generations.hash(state);
    }
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

    /// Clears all cached plans from both L1 and L2 tiers.
    ///
    /// This does **not** reset the hit/miss metrics counters.
    pub fn clear(&self) {
        self.cache.clear();
    }
}
