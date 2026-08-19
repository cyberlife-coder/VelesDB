//! Query cache for `VelesQL` parsed queries.
//!
//! Provides an LRU cache for parsed AST to avoid re-parsing identical queries.
//! Effective on workloads that repeat the **exact same query text**: a hit
//! requires equality of the original query string, so two formatting variants
//! of the same query (different whitespace, casing, parameter names) never
//! share a cache entry.

use parking_lot::RwLock;
use rustc_hash::FxHashMap;
use std::collections::VecDeque;
use std::hash::{BuildHasher, Hasher};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

use super::ast::Query;
use super::error::ParseError;
use super::Parser;

/// Statistics for the query cache.
#[derive(Debug, Clone, Copy, Default)]
pub struct CacheStats {
    /// Number of cache hits.
    pub hits: u64,
    /// Number of cache misses.
    pub misses: u64,
    /// Number of evictions.
    pub evictions: u64,
}

impl CacheStats {
    /// Returns the cache hit rate as a percentage (0.0 - 100.0).
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            return 0.0;
        }
        (self.hits as f64 / total as f64) * 100.0
    }
}

/// Bounded query cache for parsed `VelesQL` queries (issue #903).
///
/// Thread-safe implementation using `parking_lot::RwLock`.
///
/// # Design notes
///
/// - Canonical query text is hashed for compact bucketing.
/// - Hash collisions are handled explicitly via a per-bucket vector, with a
///   strict equality check on the original query text before reuse.
/// - Parsed ASTs are stored behind `Arc<Query>`; a hit returns `Arc::clone`
///   (a refcount bump) instead of deep-cloning the AST.
/// - A live `usize` size counter (`AtomicUsize`) gives O(1) `len()` and avoids
///   re-summing every bucket on each insert/eviction.
///
/// # Hot-path concurrency (issue #903)
///
/// The previous design took a **global write lock** (`order.write()`) on every
/// cache hit to promote the entry to the MRU position, serialising all reads.
/// This implementation replaces strict LRU with a **CLOCK / second-chance**
/// policy:
///
/// - A cache **hit** takes only a shared `read()` lock and sets a per-entry
///   `referenced` bit via a relaxed atomic store — no write lock, so concurrent
///   hits run in parallel.
/// - Eviction (on the cold insert path, under the write lock) sweeps the
///   insertion-order ring: an entry whose `referenced` bit is set gets a second
///   chance (bit cleared, moved to the back); an entry with a clear bit is
///   evicted. This approximates LRU while keeping the hit path lock-light.
pub struct QueryCache {
    /// Cache storage + CLOCK ring guarded by a single lock so a hit can observe
    /// both under one `read()` acquisition.
    inner: RwLock<CacheInner>,
    /// Live entry count. O(1) `len()`; kept in sync with `inner` under the write
    /// lock on insert/evict and reset on clear.
    size: AtomicUsize,
    /// Maximum cache size.
    max_size: usize,
    /// Hash function for canonical query text.
    hash_fn: fn(&str) -> u64,
    /// Cache statistics.
    stats: AtomicCacheStats,
}

/// Storage + CLOCK ring, guarded together by `QueryCache::inner`.
struct CacheInner {
    /// Cache storage: canonical-hash -> collision-safe entries.
    map: FxHashMap<u64, Vec<CacheEntry>>,
    /// CLOCK ring of cache keys in insertion order; the eviction hand sweeps
    /// from the front. Mutated only under the write lock (insert / evict).
    order: VecDeque<CacheKey>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CacheKey {
    hash: u64,
    original_query: String,
}

#[derive(Debug)]
struct CacheEntry {
    original_query: String,
    canonical_query: String,
    /// Parsed AST shared via `Arc`; hits return `Arc::clone`, never a deep copy.
    parsed: Arc<Query>,
    /// CLOCK second-chance bit. Set on every hit (relaxed atomic under a read
    /// lock); consulted and cleared by the eviction sweep.
    referenced: AtomicBool,
}

#[derive(Debug, Default)]
struct AtomicCacheStats {
    hits: AtomicU64,
    misses: AtomicU64,
    evictions: AtomicU64,
}

impl AtomicCacheStats {
    fn snapshot(&self) -> CacheStats {
        CacheStats {
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
            evictions: self.evictions.load(Ordering::Relaxed),
        }
    }

    fn clear(&self) {
        self.hits.store(0, Ordering::Relaxed);
        self.misses.store(0, Ordering::Relaxed);
        self.evictions.store(0, Ordering::Relaxed);
    }
}

impl QueryCache {
    /// Creates a new query cache with the specified maximum size.
    ///
    /// # Arguments
    ///
    /// * `max_size` - Maximum number of queries to cache (minimum 1).
    #[must_use]
    pub fn new(max_size: usize) -> Self {
        Self::new_with_hasher(max_size, default_query_hash)
    }

    fn new_with_hasher(max_size: usize, hash_fn: fn(&str) -> u64) -> Self {
        let max_size = max_size.max(1);
        Self {
            inner: RwLock::new(CacheInner {
                map: FxHashMap::default(),
                order: VecDeque::with_capacity(max_size),
            }),
            size: AtomicUsize::new(0),
            max_size,
            hash_fn,
            stats: AtomicCacheStats::default(),
        }
    }

    /// Parses a query, returning a shared (`Arc`) cached AST if available.
    ///
    /// # Errors
    ///
    /// Returns `ParseError` if the query is invalid.
    pub fn parse(&self, query: &str) -> Result<Arc<Query>, ParseError> {
        self.parse_impl(query, true)
    }

    #[cfg(feature = "internal-bench")]
    pub(crate) fn parse_without_stats(&self, query: &str) -> Result<Arc<Query>, ParseError> {
        self.parse_impl(query, false)
    }

    fn parse_impl(&self, query: &str, record_stats: bool) -> Result<Arc<Query>, ParseError> {
        let canonical_query = canonicalize_query(query);
        let hash = (self.hash_fn)(&canonical_query);

        if let Some(cached) = self.try_cache_hit(hash, query, &canonical_query, record_stats) {
            return Ok(cached);
        }

        let parsed = Arc::new(Parser::parse(query)?);
        self.insert_into_cache(hash, canonical_query, query, &parsed, record_stats);
        Ok(parsed)
    }

    /// Read-only hot path (issue #903): looks up a cached query under a **shared**
    /// lock and, on a hit, sets the CLOCK `referenced` bit with a relaxed atomic
    /// store. No write lock is taken, so concurrent hits do not serialise.
    fn try_cache_hit(
        &self,
        hash: u64,
        original_query: &str,
        canonical_query: &str,
        record_stats: bool,
    ) -> Option<Arc<Query>> {
        let inner = self.inner.read();
        let entry = inner.map.get(&hash).and_then(|entries| {
            entries.iter().find(|entry| {
                entry.original_query == original_query && entry.canonical_query == canonical_query
            })
        })?;

        // Second-chance bit: cheap relaxed store, safe under a shared lock via
        // interior mutability (AtomicBool). No global write lock on the hit path.
        entry.referenced.store(true, Ordering::Relaxed);
        let parsed = Arc::clone(&entry.parsed);
        drop(inner);

        if record_stats {
            self.stats.hits.fetch_add(1, Ordering::Relaxed);
        }
        Some(parsed)
    }

    /// Inserts a freshly parsed query into the cache, evicting via CLOCK as needed.
    fn insert_into_cache(
        &self,
        hash: u64,
        canonical_query: String,
        raw_query: &str,
        parsed: &Arc<Query>,
        record_stats: bool,
    ) {
        let mut inner = self.inner.write();

        if record_stats {
            self.stats.misses.fetch_add(1, Ordering::Relaxed);
        }

        let key = CacheKey {
            hash,
            original_query: raw_query.to_string(),
        };

        // Replacing an existing entry for the same query is not a net size change,
        // so only evict when inserting a genuinely new key.
        let is_new_key = !Self::bucket_contains(&inner.map, hash, raw_query);
        if is_new_key {
            self.evict_until_below_bound(&mut inner, record_stats);
        }

        let new_entry = CacheEntry {
            original_query: raw_query.to_string(),
            canonical_query,
            parsed: Arc::clone(parsed),
            referenced: AtomicBool::new(false),
        };

        let bucket = inner.map.entry(hash).or_default();
        bucket.retain(|entry| entry.original_query != raw_query);
        bucket.push(new_entry);

        if is_new_key {
            inner.order.push_back(key);
            self.size.fetch_add(1, Ordering::Relaxed);
        }
        debug_assert_eq!(self.size.load(Ordering::Relaxed), inner.order.len());
    }

    /// CLOCK / second-chance eviction: sweep the insertion-order ring until the
    /// live size is back under `max_size`. An entry whose `referenced` bit is set
    /// gets a second chance (bit cleared, re-queued at the back); otherwise it is
    /// evicted. Amortised O(1) per insert — no per-iteration bucket re-sum.
    fn evict_until_below_bound(&self, inner: &mut CacheInner, record_stats: bool) {
        while self.size.load(Ordering::Relaxed) >= self.max_size {
            let Some(candidate) = inner.order.pop_front() else {
                break;
            };
            if Self::take_second_chance(&inner.map, &candidate) {
                inner.order.push_back(candidate);
                continue;
            }
            Self::remove_entry(&mut inner.map, &candidate);
            self.size.fetch_sub(1, Ordering::Relaxed);
            if record_stats {
                self.stats.evictions.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Returns `true` (granting a second chance) if the candidate's entry has its
    /// `referenced` bit set, clearing the bit as a side effect.
    fn take_second_chance(map: &FxHashMap<u64, Vec<CacheEntry>>, key: &CacheKey) -> bool {
        map.get(&key.hash)
            .and_then(|bucket| {
                bucket
                    .iter()
                    .find(|entry| entry.original_query == key.original_query)
            })
            .is_some_and(|entry| entry.referenced.swap(false, Ordering::Relaxed))
    }

    /// Removes the entry identified by `key` from its bucket, dropping the bucket
    /// if it becomes empty.
    fn remove_entry(map: &mut FxHashMap<u64, Vec<CacheEntry>>, key: &CacheKey) {
        if let Some(bucket) = map.get_mut(&key.hash) {
            bucket.retain(|entry| entry.original_query != key.original_query);
            if bucket.is_empty() {
                map.remove(&key.hash);
            }
        }
    }

    /// Returns `true` if a bucket already holds an entry for `raw_query`.
    fn bucket_contains(map: &FxHashMap<u64, Vec<CacheEntry>>, hash: u64, raw_query: &str) -> bool {
        map.get(&hash)
            .is_some_and(|bucket| bucket.iter().any(|entry| entry.original_query == raw_query))
    }

    /// Returns current cache statistics.
    #[must_use]
    pub fn stats(&self) -> CacheStats {
        self.stats.snapshot()
    }

    /// Returns the current number of cached queries (O(1)).
    #[must_use]
    pub fn len(&self) -> usize {
        self.size.load(Ordering::Relaxed)
    }

    /// Returns true if the cache is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Clears all cached queries and resets statistics.
    pub fn clear(&self) {
        let mut inner = self.inner.write();
        inner.map.clear();
        inner.order.clear();
        self.size.store(0, Ordering::Relaxed);
        self.stats.clear();
    }
}

impl Default for QueryCache {
    fn default() -> Self {
        Self::new(1000)
    }
}

fn default_query_hash(query: &str) -> u64 {
    let mut hasher = rustc_hash::FxBuildHasher.build_hasher();
    hasher.write(query.as_bytes());
    hasher.finish()
}

fn canonicalize_query(query: &str) -> String {
    query.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
#[path = "cache_unit_tests.rs"]
mod tests;
