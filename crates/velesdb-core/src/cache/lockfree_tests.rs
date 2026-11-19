//! Tests for `lockfree` module - Lock-free LRU cache.

use super::lockfree::*;
use std::sync::Arc;
use std::thread;

#[test]
fn test_lockfree_cache_new() {
    let cache: LockFreeLruCache<u64, String> = LockFreeLruCache::new(100, 1000);

    assert_eq!(cache.l1_capacity(), 100);
    assert_eq!(cache.l2_capacity(), 1000);
    assert_eq!(cache.stats().l1_size, 0);
    assert_eq!(cache.stats().l2_size, 0);
}

#[test]
fn test_lockfree_cache_insert_and_get() {
    let cache: LockFreeLruCache<u64, String> = LockFreeLruCache::new(100, 1000);

    cache.insert(1, "value_1".to_string());

    assert_eq!(cache.get(&1), Some("value_1".to_string()));
    assert_eq!(cache.peek_l1(&1), Some("value_1".to_string()));
    assert_eq!(cache.peek_l2(&1), Some("value_1".to_string()));
}

#[test]
fn test_lockfree_cache_get_l1_hit() {
    let cache: LockFreeLruCache<u64, String> = LockFreeLruCache::new(100, 1000);

    cache.insert(1, "value_1".to_string());

    let _ = cache.get(&1);
    let _ = cache.get(&1);

    let stats = cache.stats();
    assert_eq!(stats.l1_hits, 2);
    assert_eq!(stats.l2_hits, 0);
    assert_eq!(stats.misses, 0);
}

#[test]
fn test_lockfree_cache_get_l2_promotion() {
    let cache: LockFreeLruCache<u64, String> = LockFreeLruCache::new(10, 100);

    cache.l2.insert(99, "from_l2".to_string());

    let result = cache.get(&99);
    assert_eq!(result, Some("from_l2".to_string()));

    let stats = cache.stats();
    assert_eq!(stats.l1_hits, 0);
    assert_eq!(stats.l2_hits, 1);

    assert!(cache.peek_l1(&99).is_some());

    let _ = cache.get(&99);
    let stats = cache.stats();
    assert_eq!(stats.l1_hits, 1);
}

#[test]
fn test_lockfree_cache_miss() {
    let cache: LockFreeLruCache<u64, String> = LockFreeLruCache::new(100, 1000);

    let result = cache.get(&999);

    assert_eq!(result, None);
    assert_eq!(cache.stats().misses, 1);
}

#[test]
fn test_lockfree_cache_remove() {
    let cache: LockFreeLruCache<u64, String> = LockFreeLruCache::new(100, 1000);

    cache.insert(1, "value_1".to_string());
    assert!(cache.get(&1).is_some());

    cache.remove(&1);

    assert!(cache.peek_l1(&1).is_none());
    assert!(cache.peek_l2(&1).is_none());
}

#[test]
fn test_lockfree_cache_clear() {
    let cache: LockFreeLruCache<u64, String> = LockFreeLruCache::new(100, 1000);

    for i in 0..50 {
        cache.insert(i, format!("value_{i}"));
    }

    assert_eq!(cache.stats().l1_size, 50);

    cache.clear();

    assert_eq!(cache.stats().l1_size, 0);
    assert_eq!(cache.stats().l2_size, 0);
}

#[test]
fn test_lockfree_cache_l1_eviction() {
    let cache: LockFreeLruCache<u64, String> = LockFreeLruCache::new(10, 100);

    for i in 0..20 {
        cache.insert(i, format!("value_{i}"));
    }

    assert_eq!(
        cache.stats().l1_size,
        cache.l1_capacity(),
        "after inserting 20 items into a capacity-10 L1, eviction must restore L1 to exactly its capacity"
    );
    assert_eq!(cache.stats().l2_size, 20);
}

#[test]
fn test_lockfree_cache_concurrent_reads() {
    let cache = Arc::new(LockFreeLruCache::<u64, String>::new(100, 1000));

    for i in 0..100 {
        cache.insert(i, format!("value_{i}"));
    }

    let mut handles = vec![];

    for _ in 0..8 {
        let cache_clone = Arc::clone(&cache);
        handles.push(thread::spawn(move || {
            for i in 0..1000 {
                let key = i % 100;
                let _ = cache_clone.get(&key);
            }
        }));
    }

    for h in handles {
        h.join().expect("Thread panicked");
    }

    let stats = cache.stats();
    assert_eq!(
        stats.l1_hits, 8000,
        "all 8x1000 reads hit pre-populated L1, got {}",
        stats.l1_hits
    );
    assert_eq!(
        stats.l2_hits, 0,
        "no read should fall through to L2, got {}",
        stats.l2_hits
    );
    assert_eq!(
        stats.misses, 0,
        "every key is present, got {} misses",
        stats.misses
    );
}

#[test]
fn test_lockfree_cache_hit_rate() {
    let cache: LockFreeLruCache<u64, String> = LockFreeLruCache::new(100, 1000);

    for i in 0..50 {
        cache.insert(i, format!("value_{i}"));
    }

    for i in 0..50 {
        let _ = cache.get(&i);
    }

    for i in 100..110 {
        let _ = cache.get(&i);
    }

    let stats = cache.stats();
    assert_eq!(stats.l1_hits, 50);
    assert_eq!(stats.misses, 10);

    let hit_rate = stats.total_hit_rate();
    assert!((hit_rate - 0.833).abs() < 0.01);
}
