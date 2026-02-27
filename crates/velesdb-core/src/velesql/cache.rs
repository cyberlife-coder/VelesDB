//! Query cache for `VelesQL` parsed queries.
//!
//! Provides an LRU cache for parsed AST to avoid re-parsing identical queries.
//! Typical cache hit rates exceed 90% on repetitive workloads.

use parking_lot::RwLock;
use rustc_hash::FxHashMap;
use std::collections::VecDeque;
use std::hash::{BuildHasher, Hasher};

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
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            (self.hits as f64 / total as f64) * 100.0
        }
    }
}

/// LRU cache for parsed `VelesQL` queries.
///
/// Thread-safe implementation using `parking_lot::RwLock`.
///
/// # Design notes
///
/// - Canonical query text is hashed for compact bucketing.
/// - Hash collisions are handled explicitly via a per-bucket vector.
/// - A strict equality check on original query text is required before reuse.
/// - LRU order stores unique cache keys and is kept in sync with entry count.
pub struct QueryCache {
    /// Cache storage: canonical-hash -> collision-safe entries.
    cache: RwLock<FxHashMap<u64, Vec<CacheEntry>>>,
    /// LRU order: front = oldest key, back = most recently used.
    order: RwLock<VecDeque<CacheKey>>,
    /// Maximum cache size.
    max_size: usize,
    /// Hash function for canonical query text.
    hash_fn: fn(&str) -> u64,
    /// Cache statistics.
    stats: RwLock<CacheStats>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CacheKey {
    hash: u64,
    original_query: String,
}

#[derive(Debug, Clone)]
struct CacheEntry {
    original_query: String,
    canonical_query: String,
    parsed: Query,
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
        Self {
            cache: RwLock::new(FxHashMap::default()),
            order: RwLock::new(VecDeque::with_capacity(max_size.max(1))),
            max_size: max_size.max(1),
            hash_fn,
            stats: RwLock::new(CacheStats::default()),
        }
    }

    /// Parses a query, returning cached AST if available.
    ///
    /// # Errors
    ///
    /// Returns `ParseError` if the query is invalid.
    pub fn parse(&self, query: &str) -> Result<Query, ParseError> {
        let canonical_query = canonicalize_query(query);
        let hash = (self.hash_fn)(&canonical_query);

        let cached = {
            self.cache.read().get(&hash).and_then(|entries| {
                entries
                    .iter()
                    .find(|entry| {
                        // Strict equality check on hit to avoid query confusion.
                        entry.original_query == query && entry.canonical_query == canonical_query
                    })
                    .cloned()
            })
        };

        if let Some(cached) = cached {
            let mut order = self.order.write();
            let mut stats = self.stats.write();

            stats.hits += 1;

            let key = CacheKey {
                hash,
                original_query: query.to_string(),
            };

            // Move to MRU, keeping order duplicate-free.
            if let Some(pos) = order.iter().position(|existing| existing == &key) {
                order.remove(pos);
            }
            order.push_back(key);

            return Ok(cached.parsed);
        }

        let parsed = Parser::parse(query)?;

        {
            let mut cache = self.cache.write();
            let mut order = self.order.write();
            let mut stats = self.stats.write();

            stats.misses += 1;

            while Self::entry_count(&cache) >= self.max_size {
                if let Some(oldest) = order.pop_front() {
                    if let Some(bucket) = cache.get_mut(&oldest.hash) {
                        bucket.retain(|entry| entry.original_query != oldest.original_query);
                        if bucket.is_empty() {
                            cache.remove(&oldest.hash);
                        }
                    }
                    stats.evictions += 1;
                }
            }

            let key = CacheKey {
                hash,
                original_query: query.to_string(),
            };

            if let Some(pos) = order.iter().position(|existing| existing == &key) {
                order.remove(pos);
            }

            let new_entry = CacheEntry {
                original_query: query.to_string(),
                canonical_query,
                parsed: parsed.clone(),
            };

            cache
                .entry(hash)
                .and_modify(|bucket| {
                    bucket.retain(|entry| entry.original_query != query);
                    bucket.push(new_entry.clone());
                })
                .or_insert_with(|| vec![new_entry]);

            order.push_back(key);

            debug_assert_eq!(Self::entry_count(&cache), order.len());
        }

        Ok(parsed)
    }

    /// Returns current cache statistics.
    #[must_use]
    pub fn stats(&self) -> CacheStats {
        *self.stats.read()
    }

    /// Returns the current number of cached queries.
    #[must_use]
    pub fn len(&self) -> usize {
        Self::entry_count(&self.cache.read())
    }

    /// Returns true if the cache is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Clears all cached queries and resets statistics.
    pub fn clear(&self) {
        let mut cache = self.cache.write();
        let mut order = self.order.write();
        let mut stats = self.stats.write();

        cache.clear();
        order.clear();
        *stats = CacheStats::default();
    }

    fn entry_count(cache: &FxHashMap<u64, Vec<CacheEntry>>) -> usize {
        cache.values().map(std::vec::Vec::len).sum()
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
mod tests {
    use super::*;

    #[test]
    fn test_cache_stats_hit_rate_empty() {
        let stats = CacheStats::default();
        assert!((stats.hit_rate() - 0.0).abs() < 1e-5);
    }

    #[test]
    fn test_cache_stats_hit_rate_all_hits() {
        let stats = CacheStats {
            hits: 10,
            misses: 0,
            evictions: 0,
        };
        assert!((stats.hit_rate() - 100.0).abs() < 1e-5);
    }

    #[test]
    fn test_cache_stats_hit_rate_half() {
        let stats = CacheStats {
            hits: 5,
            misses: 5,
            evictions: 0,
        };
        assert!((stats.hit_rate() - 50.0).abs() < 1e-5);
    }

    #[test]
    fn test_query_cache_new() {
        let cache = QueryCache::new(100);
        assert!(cache.is_empty());
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn test_query_cache_default() {
        let cache = QueryCache::default();
        assert!(cache.is_empty());
    }

    #[test]
    fn test_query_cache_parse_and_hit() {
        let cache = QueryCache::new(10);
        let query = "SELECT * FROM docs LIMIT 5";

        let result1 = cache.parse(query);
        assert!(result1.is_ok());
        assert_eq!(cache.stats().misses, 1);
        assert_eq!(cache.stats().hits, 0);

        let result2 = cache.parse(query);
        assert!(result2.is_ok());
        assert_eq!(cache.stats().hits, 1);
    }

    #[test]
    fn test_query_cache_clear() {
        let cache = QueryCache::new(10);
        let _ = cache.parse("SELECT * FROM docs LIMIT 1");
        assert!(!cache.is_empty());

        cache.clear();
        assert!(cache.is_empty());
        assert_eq!(cache.stats().hits, 0);
        assert_eq!(cache.stats().misses, 0);
    }

    #[test]
    fn test_query_cache_eviction() {
        let cache = QueryCache::new(2);

        let _ = cache.parse("SELECT * FROM docs LIMIT 1");
        let _ = cache.parse("SELECT * FROM docs LIMIT 2");
        assert_eq!(cache.len(), 2);

        let _ = cache.parse("SELECT * FROM docs LIMIT 3");
        assert_eq!(cache.len(), 2);
        assert!(cache.stats().evictions >= 1);
    }

    #[test]
    fn test_query_cache_hit_refreshes_mru_without_duplicates() {
        let cache = QueryCache::new(3);
        let q1 = "SELECT * FROM docs LIMIT 1";
        let q2 = "SELECT * FROM docs LIMIT 2";
        let q3 = "SELECT * FROM docs LIMIT 3";

        let _ = cache.parse(q1);
        let _ = cache.parse(q2);
        let _ = cache.parse(q3);
        let _ = cache.parse(q1);

        let order = cache.order.read();
        assert_eq!(order.len(), cache.len());
        assert_eq!(
            order
                .iter()
                .filter(|v| v.original_query.as_str() == q1)
                .count(),
            1
        );
        assert_eq!(order.back().map(|v| v.original_query.as_str()), Some(q1));
    }

    #[test]
    fn test_query_cache_concurrent_invariant_no_order_duplicates() {
        use std::sync::Arc;
        use std::thread;

        let cache = Arc::new(QueryCache::new(32));
        let queries = [
            "SELECT * FROM docs LIMIT 1",
            "SELECT * FROM docs LIMIT 2",
            "SELECT * FROM docs LIMIT 3",
            "SELECT * FROM docs LIMIT 4",
            "SELECT * FROM docs LIMIT 5",
        ];

        let mut handles = Vec::new();
        for _ in 0..8 {
            let cache = Arc::clone(&cache);
            handles.push(thread::spawn(move || {
                for i in 0..200 {
                    let q = queries[i % queries.len()];
                    let _ = cache.parse(q);
                }
            }));
        }

        for h in handles {
            h.join().expect("thread must complete");
        }

        let order = cache.order.read();
        let mut uniq = std::collections::HashSet::new();
        for key in order.iter() {
            assert!(uniq.insert(key.clone()), "duplicate query in LRU order");
        }
        assert_eq!(order.len(), cache.len());
    }

    #[test]
    fn test_query_cache_collision_safe_with_forced_hash_collision() {
        let cache = QueryCache::new_with_hasher(10, |_| 42);
        let q1 = "SELECT * FROM docs LIMIT 1";
        let q2 = "SELECT id FROM docs LIMIT 2";

        let r1 = cache.parse(q1).expect("q1 should parse");
        let r2 = cache.parse(q2).expect("q2 should parse");
        let r1_again = cache.parse(q1).expect("q1 should be cache hit");

        assert_eq!(r1, r1_again);
        assert_ne!(r1, r2);
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn test_query_cache_min_size() {
        let cache = QueryCache::new(0);
        let _ = cache.parse("SELECT * FROM docs LIMIT 1");
        assert!(!cache.is_empty());
    }

    #[test]
    fn test_query_cache_invalid_query() {
        let cache = QueryCache::new(10);
        let result = cache.parse("INVALID QUERY SYNTAX!!!");
        assert!(result.is_err());
    }
}
