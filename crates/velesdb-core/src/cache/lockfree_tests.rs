#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::float_cmp
)]
//! Tests for `lockfree` module - Lock-free LRU cache.

use super::lockfree::*;
use std::sync::Arc;
use std::thread;
use std::time::Instant;

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
fn test_lockfree_cache_remove_preserves_l2_recency_order() {
    let cache: LockFreeLruCache<u64, String> = LockFreeLruCache::new(100, 3);

    cache.insert(1, "value_1".to_string());
    cache.insert(2, "value_2".to_string());
    cache.insert(3, "value_3".to_string());

    cache.remove(&1);
    cache.insert(4, "value_4".to_string());
    cache.insert(5, "value_5".to_string());

    // L2 is the capacity tier: its eviction must pick 2, the oldest survivor.
    assert!(
        cache.peek_l2(&2).is_none(),
        "the oldest L2 entry must be evicted"
    );
    assert!(cache.peek_l2(&3).is_some());
    assert!(cache.peek_l2(&4).is_some());
    assert!(cache.peek_l2(&5).is_some());
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

/// L1 must evict its least recently used entry, whatever the `DashMap`
/// iteration order. Each round spares every key but one; an eviction that
/// ignores recency picks the same key in every round and fails the others.
#[test]
fn test_lockfree_cache_l1_evicts_least_recently_used() {
    const L1_CAPACITY: u64 = 8;

    for victim in 0..L1_CAPACITY {
        let cache: LockFreeLruCache<u64, String> = LockFreeLruCache::new(8, 100);
        for key in 0..L1_CAPACITY {
            cache.insert(key, format!("value_{key}"));
        }
        for key in (0..L1_CAPACITY).filter(|&key| key != victim) {
            assert!(cache.get(&key).is_some());
        }

        cache.insert(L1_CAPACITY, "newcomer".to_string());

        assert!(
            cache.peek_l1(&victim).is_none(),
            "round {victim}: the least recently used key must leave L1"
        );
        for key in (0..=L1_CAPACITY).filter(|&key| key != victim) {
            assert!(
                cache.peek_l1(&key).is_some(),
                "round {victim}: key {key} was used more recently than the victim"
            );
        }
    }
}

/// An insert is more recent than every access before it, so the entry it
/// adds is never its own eviction victim, even when every other L1 entry
/// was hit since the previous insert. Many newcomer keys vary the hash
/// position of the newcomer, so an insert stamped no newer than those hits
/// is caught.
#[test]
fn test_lockfree_cache_l1_never_evicts_the_entry_just_inserted() {
    for newcomer in 4..68_u64 {
        let cache: LockFreeLruCache<u64, u64> = LockFreeLruCache::new(4, 100);
        for key in 0..4 {
            cache.insert(key, key);
        }
        for key in 0..4 {
            assert!(cache.get(&key).is_some());
        }

        cache.insert(newcomer, newcomer);

        assert!(
            cache.peek_l1(&newcomer).is_some(),
            "newcomer {newcomer} was evicted by its own insert"
        );
    }
}

#[test]
fn test_lockfree_cache_l2_promotion_refreshes_l1_recency() {
    let cache: LockFreeLruCache<u64, String> = LockFreeLruCache::new(2, 100);
    cache.insert(1, "value_1".to_string());
    cache.insert(2, "value_2".to_string());
    cache.insert(3, "value_3".to_string()); // L1 = {2, 3}; 1 is in L2 only.

    // L2 hit: 1 is promoted as the most recent L1 entry, so 2 is the victim.
    assert!(cache.get(&1).is_some());

    assert!(
        cache.peek_l1(&1).is_some(),
        "a promoted key is the MRU entry"
    );
    assert!(cache.peek_l1(&2).is_none());
    assert!(cache.peek_l1(&3).is_some());
}

#[test]
fn test_lockfree_cache_peek_l1_does_not_refresh_recency() {
    let cache: LockFreeLruCache<u64, String> = LockFreeLruCache::new(2, 100);
    cache.insert(1, "value_1".to_string());
    cache.insert(2, "value_2".to_string());

    assert!(cache.peek_l1(&1).is_some());
    cache.insert(3, "value_3".to_string());

    assert!(
        cache.peek_l1(&1).is_none(),
        "peek must leave 1 the LRU entry"
    );
    assert!(cache.peek_l1(&2).is_some());
    assert!(cache.peek_l1(&3).is_some());
}

/// Concurrent hits racing L1 evictions: every hit must return the value
/// stored under its own key, and any excess left in L1 once the writers are
/// done must have been reported in `l1_overflow`. The last writer to grow L1
/// checks it after its eviction passes, so an excess it leaves is counted.
#[test]
fn test_lockfree_cache_concurrent_l1_eviction_keeps_entries_consistent() {
    const THREADS: u64 = 8;
    const L1_CAPACITY: usize = 16;
    let cache = Arc::new(LockFreeLruCache::<u64, u64>::new(L1_CAPACITY, 1_000));

    let handles: Vec<_> = (0..THREADS)
        .map(|t| {
            let cache = Arc::clone(&cache);
            thread::spawn(move || {
                for i in 0..2_000_u64 {
                    let key = (i * THREADS + t) % 64;
                    if i % 3 == 0 {
                        cache.insert(key, key * 10);
                    } else if let Some(value) = cache.get(&key) {
                        assert_eq!(value, key * 10, "key {key} returned another entry");
                    }
                }
            })
        })
        .collect();
    for handle in handles {
        handle.join().expect("worker panicked");
    }

    let stats = cache.stats();
    assert!(
        stats.l1_size <= L1_CAPACITY || stats.l1_overflow > 0,
        "L1 holds {} entries for a capacity of {L1_CAPACITY}, with no overflow reported",
        stats.l1_size
    );
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
#[ignore = "Performance test - run manually with --ignored, CI runners are too slow"]
fn test_lockfree_cache_scaling_8_threads() {
    let cache = Arc::new(LockFreeLruCache::<u64, String>::new(1000, 10000));

    for i in 0..1000 {
        cache.insert(i, format!("value_{i}"));
    }

    let ops_per_thread = 5_000;

    let start = Instant::now();
    for i in 0..ops_per_thread {
        let _ = cache.get(&(i % 1000));
    }
    let single_thread_time = start.elapsed();

    let start = Instant::now();
    let mut handles = vec![];
    for t in 0..8 {
        let cache_clone = Arc::clone(&cache);
        handles.push(thread::spawn(move || {
            let offset = t * 100;
            for i in 0..ops_per_thread {
                let key = ((i + offset) % 1000) as u64;
                let _ = cache_clone.get(&key);
            }
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
    let eight_thread_time = start.elapsed();

    let single_throughput = ops_per_thread as f64 / single_thread_time.as_secs_f64();
    let eight_throughput = (8 * ops_per_thread) as f64 / eight_thread_time.as_secs_f64();
    let scaling_factor = eight_throughput / single_throughput;

    println!("LockFreeLruCache scaling test:");
    println!("  1 thread:  {:.0} ops/sec", single_throughput);
    println!("  8 threads: {:.0} ops/sec", eight_throughput);
    println!("  Scaling:   {:.2}x", scaling_factor);

    assert!(
        scaling_factor > 0.5,
        "Scaling factor {scaling_factor:.2}x should be > 0.5x (no severe regression)"
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
