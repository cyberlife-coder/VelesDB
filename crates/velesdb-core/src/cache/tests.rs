//! TDD Tests for LRU Cache (US-CORE-003-04)

use super::*;

// ========== LRU Cache Basic Tests ==========

#[test]
fn test_lru_cache_new() {
    let cache: LruCache<u64, String> = LruCache::new(100);
    assert!(cache.is_empty());
    assert_eq!(cache.len(), 0);
    assert_eq!(cache.capacity(), 100);
}

#[test]
fn test_lru_cache_insert_and_get() {
    let cache: LruCache<u64, String> = LruCache::new(100);

    cache.insert(1, "hello".to_string());

    assert_eq!(cache.get(&1), Some("hello".to_string()));
    assert!(!cache.is_empty());
    assert_eq!(cache.len(), 1);
}

#[test]
fn test_lru_cache_get_nonexistent() {
    let cache: LruCache<u64, String> = LruCache::new(100);

    assert_eq!(cache.get(&999), None);
}

#[test]
fn test_lru_cache_update_existing() {
    let cache: LruCache<u64, String> = LruCache::new(100);

    cache.insert(1, "hello".to_string());
    cache.insert(1, "world".to_string());

    assert_eq!(cache.get(&1), Some("world".to_string()));
    assert_eq!(cache.len(), 1);
}

#[test]
fn test_lru_cache_remove() {
    let cache: LruCache<u64, String> = LruCache::new(100);

    cache.insert(1, "hello".to_string());
    cache.remove(&1);

    assert_eq!(cache.get(&1), None);
    assert!(cache.is_empty());
}

// ========== LRU Eviction Tests ==========

#[test]
fn test_lru_cache_eviction_when_full() {
    let cache: LruCache<u64, String> = LruCache::new(3);

    cache.insert(1, "one".to_string());
    cache.insert(2, "two".to_string());
    cache.insert(3, "three".to_string());

    // Cache is full, inserting 4 should evict 1 (LRU)
    cache.insert(4, "four".to_string());

    assert_eq!(cache.get(&1), None); // Evicted
    assert_eq!(cache.get(&2), Some("two".to_string()));
    assert_eq!(cache.get(&3), Some("three".to_string()));
    assert_eq!(cache.get(&4), Some("four".to_string()));
}

#[test]
fn test_lru_cache_access_updates_recency() {
    let cache: LruCache<u64, String> = LruCache::new(3);

    cache.insert(1, "one".to_string());
    cache.insert(2, "two".to_string());
    cache.insert(3, "three".to_string());

    // Access 1 to make it recently used
    let _ = cache.get(&1);

    // Insert 4 should evict 2 (now LRU)
    cache.insert(4, "four".to_string());

    assert_eq!(cache.get(&1), Some("one".to_string())); // Still there
    assert_eq!(cache.get(&2), None); // Evicted
    assert_eq!(cache.get(&3), Some("three".to_string()));
    assert_eq!(cache.get(&4), Some("four".to_string()));
}

// ========== Cache Stats Tests ==========

#[test]
fn test_lru_cache_stats_hits() {
    let cache: LruCache<u64, String> = LruCache::new(100);

    cache.insert(1, "hello".to_string());

    let _ = cache.get(&1); // Hit
    let _ = cache.get(&1); // Hit

    let stats = cache.stats();
    assert_eq!(stats.hits, 2);
}

#[test]
fn test_lru_cache_stats_misses() {
    let cache: LruCache<u64, String> = LruCache::new(100);

    let _ = cache.get(&1); // Miss
    let _ = cache.get(&2); // Miss

    let stats = cache.stats();
    assert_eq!(stats.misses, 2);
}

#[test]
fn test_lru_cache_stats_evictions() {
    let cache: LruCache<u64, String> = LruCache::new(2);

    cache.insert(1, "one".to_string());
    cache.insert(2, "two".to_string());
    cache.insert(3, "three".to_string()); // Evicts 1
    cache.insert(4, "four".to_string()); // Evicts 2

    let stats = cache.stats();
    assert_eq!(stats.evictions, 2);
}

#[test]
fn test_lru_cache_hit_rate() {
    let cache: LruCache<u64, String> = LruCache::new(100);

    cache.insert(1, "hello".to_string());

    let _ = cache.get(&1); // Hit
    let _ = cache.get(&1); // Hit
    let _ = cache.get(&2); // Miss
    let _ = cache.get(&3); // Miss

    let stats = cache.stats();
    assert!((stats.hit_rate() - 0.5).abs() < 0.01); // 2 hits / 4 total
}

// ========== Thread Safety Tests ==========

#[test]
fn test_lru_cache_concurrent_access() {
    use std::sync::Arc;
    use std::thread;

    let cache: Arc<LruCache<u64, String>> = Arc::new(LruCache::new(1000));

    let mut handles = vec![];

    // Spawn 4 threads doing concurrent inserts
    for t in 0..4 {
        let cache_clone = Arc::clone(&cache);
        handles.push(thread::spawn(move || {
            for i in 0..100 {
                let key = t * 100 + i;
                cache_clone.insert(key, format!("value_{key}"));
            }
        }));
    }

    for handle in handles {
        handle.join().unwrap();
    }

    // All inserts should succeed (some may be evicted)
    assert!(cache.len() <= 1000);
}

#[test]
fn test_lru_cache_clear() {
    let cache: LruCache<u64, String> = LruCache::new(100);

    cache.insert(1, "one".to_string());
    cache.insert(2, "two".to_string());

    cache.clear();

    assert!(cache.is_empty());
    assert_eq!(cache.get(&1), None);
    assert_eq!(cache.get(&2), None);
}
