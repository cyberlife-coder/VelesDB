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

/// A key served only from L1 never moves in L2, so L2 may evict it while it
/// is hot. When L1 evicts it in turn, it must be demoted to L2's MRU
/// position rather than lost: L2 then holds the most recently used keys.
#[test]
fn test_lockfree_cache_l1_victim_is_demoted_to_l2_mru() {
    let cache: LockFreeLruCache<u64, String> = LockFreeLruCache::new(2, 3);
    cache.insert(1, "value_1".to_string());
    cache.insert(2, "value_2".to_string());
    for newcomer in 3..=5 {
        // 1 stays hot in L1 only, while write-through traffic ages it in L2.
        assert!(cache.get(&1).is_some());
        cache.insert(newcomer, format!("value_{newcomer}"));
    }
    assert!(cache.peek_l2(&1).is_none(), "setup: L2 evicted the hot key");

    // 1 is now L1's LRU entry: inserting 6 evicts it from L1.
    cache.insert(6, "value_6".to_string());

    assert!(cache.peek_l1(&1).is_none());
    // Accesses, most recent first: 6, 5, 1, 4, 3, 2. L2 keeps the top three.
    for key in [1, 5, 6] {
        assert!(cache.peek_l2(&key).is_some(), "L2 must hold key {key}");
    }
    assert!(cache.peek_l2(&4).is_none());
}

/// Demotion must not resurrect a key that a concurrent `remove` deleted:
/// once `remove` returns and no writer re-inserts that key, it is in
/// neither tier.
#[test]
fn test_lockfree_cache_remove_racing_l1_demotion_does_not_resurrect() {
    const VICTIMS: u64 = 8;
    for round in 0..200_u64 {
        let cache = Arc::new(LockFreeLruCache::<u64, u64>::new(8, 10_000));
        for key in 0..VICTIMS {
            cache.insert(key, key);
        }

        let writers: Vec<_> = (0..4_u64)
            .map(|w| {
                let cache = Arc::clone(&cache);
                thread::spawn(move || {
                    // Fresh keys only: every insert evicts, and so demotes,
                    // an older L1 entry, possibly a victim being removed.
                    for i in 0..16 {
                        let key = 1_000 + (w * 100) + i;
                        cache.insert(key, key);
                    }
                })
            })
            .collect();
        let remover = {
            let cache = Arc::clone(&cache);
            thread::spawn(move || {
                for key in 0..VICTIMS {
                    cache.remove(&key);
                }
            })
        };
        for handle in writers {
            handle.join().expect("writer panicked");
        }
        remover.join().expect("remover panicked");

        for key in 0..VICTIMS {
            assert!(
                cache.peek_l1(&key).is_none() && cache.peek_l2(&key).is_none(),
                "round {round}: removed key {key} came back"
            );
        }
    }
}

/// A cache whose keys `0..VICTIMS` sit in L2 only: inserted, then pushed
/// out of an L1 of capacity `VICTIMS` by as many fresh keys.
fn cache_with_victims_in_l2_only() -> Arc<LockFreeLruCache<u64, u64>> {
    let cache = Arc::new(LockFreeLruCache::new(VICTIMS as usize, 10_000));
    for key in 0..VICTIMS {
        cache.insert(key, key);
    }
    for key in 1_000..1_000 + VICTIMS {
        cache.insert(key, key);
    }
    cache
}

const VICTIMS: u64 = 8;

/// A promotion must not resurrect a key that a concurrent `remove`
/// deleted: a `get` that read the key from L2 before the removal must not
/// put it back into L1 after the removal returned.
#[test]
fn test_lockfree_cache_remove_racing_l2_promotion_does_not_resurrect() {
    for round in 0..500_u64 {
        let cache = cache_with_victims_in_l2_only();
        let readers: Vec<_> = (0..3)
            .map(|_| {
                let cache = Arc::clone(&cache);
                thread::spawn(move || {
                    for key in (0..VICTIMS).cycle().take(64) {
                        let _ = cache.get(&key);
                    }
                })
            })
            .collect();
        for key in 0..VICTIMS {
            cache.remove(&key);
        }
        for handle in readers {
            handle.join().expect("reader panicked");
        }

        for key in 0..VICTIMS {
            assert!(
                cache.peek_l1(&key).is_none() && cache.peek_l2(&key).is_none(),
                "round {round}: removed key {key} was promoted back"
            );
        }
    }
}

/// A promotion must not overwrite a newer insert: once `insert(k, new)`
/// has returned, a `get` that read the old value from L2 before it must
/// not put that old value into L1.
#[test]
fn test_lockfree_cache_promotion_never_overwrites_a_newer_insert() {
    for round in 0..500_u64 {
        let cache = cache_with_victims_in_l2_only();
        let readers: Vec<_> = (0..3)
            .map(|_| {
                let cache = Arc::clone(&cache);
                thread::spawn(move || {
                    for key in (0..VICTIMS).cycle().take(64) {
                        let _ = cache.get(&key);
                    }
                })
            })
            .collect();
        for key in 0..VICTIMS {
            cache.insert(key, key + 100);
        }
        for handle in readers {
            handle.join().expect("reader panicked");
        }

        for key in 0..VICTIMS {
            assert_eq!(
                cache.get(&key),
                Some(key + 100),
                "round {round}: key {key} lost its newer value to a promotion"
            );
        }
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
