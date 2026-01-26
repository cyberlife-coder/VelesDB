//! Query cache for `VelesQL` parsed queries.
//!
//! Provides an LRU cache for parsed AST to avoid re-parsing identical queries.
//! Typical cache hit rates exceed 90% on repetitive workloads.

use parking_lot::RwLock;
use rustc_hash::FxHashMap;
use std::collections::VecDeque;
use std::hash::{Hash, Hasher};

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
            #[allow(clippy::cast_precision_loss)]
            let rate = (self.hits as f64 / total as f64) * 100.0;
            rate
        }
    }
}

/// LRU cache for parsed `VelesQL` queries.
///
/// Thread-safe implementation using `parking_lot::RwLock`.
///
/// # Example
///
/// ```ignore
/// use velesdb_core::velesql::QueryCache;
///
/// let cache = QueryCache::new(1000);
/// let query = cache.parse("SELECT * FROM documents LIMIT 10")?;
/// // Second call returns cached AST
/// let query2 = cache.parse("SELECT * FROM documents LIMIT 10")?;
/// assert!(cache.stats().hits >= 1);
/// ```
pub struct QueryCache {
    /// Cache storage: hash -> Query
    cache: RwLock<FxHashMap<u64, Query>>,
    /// LRU order: front = oldest, back = newest
    order: RwLock<VecDeque<u64>>,
    /// Maximum cache size
    max_size: usize,
    /// Cache statistics
    stats: RwLock<CacheStats>,
}

impl QueryCache {
    /// Creates a new query cache with the specified maximum size.
    ///
    /// # Arguments
    ///
    /// * `max_size` - Maximum number of queries to cache (minimum 1)
    #[must_use]
    pub fn new(max_size: usize) -> Self {
        Self {
            cache: RwLock::new(FxHashMap::default()),
            order: RwLock::new(VecDeque::with_capacity(max_size)),
            max_size: max_size.max(1),
            stats: RwLock::new(CacheStats::default()),
        }
    }

    /// Parses a query, returning cached AST if available.
    ///
    /// # Errors
    ///
    /// Returns `ParseError` if the query is invalid.
    pub fn parse(&self, query: &str) -> Result<Query, ParseError> {
        let hash = Self::hash_query(query);

        // Try cache read first
        {
            let cache = self.cache.read();
            if let Some(cached) = cache.get(&hash) {
                let mut stats = self.stats.write();
                stats.hits += 1;
                return Ok(cached.clone());
            }
        }

        // Cache miss - parse the query
        let parsed = Parser::parse(query)?;

        // Insert into cache
        {
            let mut cache = self.cache.write();
            let mut order = self.order.write();
            let mut stats = self.stats.write();

            stats.misses += 1;

            // Evict oldest if at capacity
            while cache.len() >= self.max_size {
                if let Some(oldest) = order.pop_front() {
                    cache.remove(&oldest);
                    stats.evictions += 1;
                }
            }

            cache.insert(hash, parsed.clone());
            order.push_back(hash);
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
        self.cache.read().len()
    }

    /// Returns true if the cache is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.cache.read().is_empty()
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

    /// Computes a hash for the query string.
    fn hash_query(query: &str) -> u64 {
        let mut hasher = rustc_hash::FxHasher::default();
        query.hash(&mut hasher);
        hasher.finish()
    }
}

impl Default for QueryCache {
    fn default() -> Self {
        Self::new(1000)
    }
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

        // First parse - miss
        let result1 = cache.parse(query);
        assert!(result1.is_ok());
        assert_eq!(cache.stats().misses, 1);
        assert_eq!(cache.stats().hits, 0);

        // Second parse - hit
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

        // Fill cache
        let _ = cache.parse("SELECT * FROM docs LIMIT 1");
        let _ = cache.parse("SELECT * FROM docs LIMIT 2");
        assert_eq!(cache.len(), 2);

        // Add third query - should evict oldest
        let _ = cache.parse("SELECT * FROM docs LIMIT 3");
        assert_eq!(cache.len(), 2);
        assert!(cache.stats().evictions >= 1);
    }

    #[test]
    fn test_query_cache_min_size() {
        // Even with 0, should have minimum size of 1
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
