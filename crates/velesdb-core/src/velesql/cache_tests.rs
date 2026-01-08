//! Tests for `cache` module

use super::*;

#[test]
fn test_cache_new() {
    // Arrange & Act
    let cache = QueryCache::new(100);

    // Assert
    assert_eq!(cache.len(), 0);
    assert!(cache.is_empty());
    assert_eq!(cache.stats().hits, 0);
    assert_eq!(cache.stats().misses, 0);
}

#[test]
fn test_cache_parse_miss() {
    // Arrange
    let cache = QueryCache::new(100);

    // Act
    let result = cache.parse("SELECT * FROM documents");

    // Assert
    assert!(result.is_ok());
    assert_eq!(cache.len(), 1);
    assert_eq!(cache.stats().misses, 1);
    assert_eq!(cache.stats().hits, 0);
}

#[test]
fn test_cache_parse_hit() {
    // Arrange
    let cache = QueryCache::new(100);
    let query = "SELECT * FROM documents LIMIT 10";

    // Act - first parse (miss)
    let result1 = cache.parse(query);
    // Act - second parse (hit)
    let result2 = cache.parse(query);

    // Assert
    assert!(result1.is_ok());
    assert!(result2.is_ok());
    assert_eq!(result1.unwrap(), result2.unwrap());
    assert_eq!(cache.stats().hits, 1);
    assert_eq!(cache.stats().misses, 1);
}

#[test]
fn test_cache_hit_rate() {
    // Arrange
    let cache = QueryCache::new(100);
    let query = "SELECT * FROM test";

    // Act - 1 miss, 9 hits
    for _ in 0..10 {
        let _ = cache.parse(query);
    }

    // Assert
    let stats = cache.stats();
    assert_eq!(stats.hits, 9);
    assert_eq!(stats.misses, 1);
    assert!((stats.hit_rate() - 90.0).abs() < 0.01);
}

#[test]
fn test_cache_eviction() {
    // Arrange
    let cache = QueryCache::new(3);

    // Act - insert 4 queries into cache of size 3
    let _ = cache.parse("SELECT * FROM a");
    let _ = cache.parse("SELECT * FROM b");
    let _ = cache.parse("SELECT * FROM c");
    let _ = cache.parse("SELECT * FROM d");

    // Assert
    assert_eq!(cache.len(), 3);
    assert_eq!(cache.stats().evictions, 1);
}

#[test]
fn test_cache_clear() {
    // Arrange
    let cache = QueryCache::new(100);
    let _ = cache.parse("SELECT * FROM test");
    let _ = cache.parse("SELECT * FROM test");

    // Act
    cache.clear();

    // Assert
    assert!(cache.is_empty());
    assert_eq!(cache.stats().hits, 0);
    assert_eq!(cache.stats().misses, 0);
}

#[test]
fn test_cache_invalid_query() {
    // Arrange
    let cache = QueryCache::new(100);

    // Act
    let result = cache.parse("INVALID QUERY");

    // Assert
    assert!(result.is_err());
    assert!(cache.is_empty()); // Invalid queries should not be cached
}

#[test]
fn test_cache_different_queries() {
    // Arrange
    let cache = QueryCache::new(100);

    // Act
    let _ = cache.parse("SELECT * FROM a");
    let _ = cache.parse("SELECT * FROM b");
    let _ = cache.parse("SELECT id FROM c WHERE id = 1");

    // Assert
    assert_eq!(cache.len(), 3);
    assert_eq!(cache.stats().misses, 3);
    assert_eq!(cache.stats().hits, 0);
}

#[test]
fn test_cache_min_size() {
    // Arrange - cache size 0 should be clamped to 1
    let cache = QueryCache::new(0);

    // Act
    let _ = cache.parse("SELECT * FROM a");
    let _ = cache.parse("SELECT * FROM b");

    // Assert
    assert_eq!(cache.len(), 1); // Only 1 entry due to min size
    assert_eq!(cache.stats().evictions, 1);
}

#[test]
fn test_cache_thread_safety() {
    use std::sync::Arc;
    use std::thread;

    // Arrange
    let cache = Arc::new(QueryCache::new(100));
    let query = "SELECT * FROM concurrent_test";

    // Act - spawn multiple threads
    let handles: Vec<_> = (0..10)
        .map(|_| {
            let cache = Arc::clone(&cache);
            let q = query.to_string();
            thread::spawn(move || {
                for _ in 0..100 {
                    let _ = cache.parse(&q);
                }
            })
        })
        .collect();

    for h in handles {
        h.join().expect("Thread panicked");
    }

    // Assert - should have high hit rate
    let stats = cache.stats();
    assert!(stats.hit_rate() > 90.0);
    assert_eq!(stats.hits + stats.misses, 1000);
}

#[test]
fn test_cache_stats_hit_rate_empty() {
    // Arrange
    let stats = CacheStats::default();

    // Act & Assert
    assert!(stats.hit_rate().abs() < f64::EPSILON);
}
