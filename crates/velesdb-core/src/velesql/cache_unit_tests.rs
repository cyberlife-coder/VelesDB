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
#[expect(clippy::significant_drop_tightening)] // Reason: the guard under test is held to the assertion on purpose
fn test_query_cache_hit_keeps_clock_ring_unique() {
    // Issue #903: a hit no longer rewrites LRU order (CLOCK promotion sets a
    // referenced bit instead). The ring must still contain each key once and
    // stay in sync with the O(1) size counter.
    let cache = QueryCache::new(3);
    let q1 = "SELECT * FROM docs LIMIT 1";
    let q2 = "SELECT * FROM docs LIMIT 2";
    let q3 = "SELECT * FROM docs LIMIT 3";

    let _ = cache.parse(q1);
    let _ = cache.parse(q2);
    let _ = cache.parse(q3);
    let _ = cache.parse(q1); // hit: sets referenced bit, no reordering

    let inner = cache.inner.read();
    assert_eq!(inner.order.len(), cache.len());
    assert_eq!(
        inner
            .order
            .iter()
            .filter(|v| v.original_query.as_str() == q1)
            .count(),
        1,
        "no duplicate ring entries on hit"
    );
}

#[test]
fn test_query_cache_clock_referenced_entry_survives_eviction() {
    // Issue #903: CLOCK second chance. q1 is referenced (hit) before pressure;
    // it must survive while an un-referenced entry is evicted instead.
    let cache = QueryCache::new(2);
    let q1 = "SELECT * FROM docs LIMIT 1";
    let q2 = "SELECT * FROM docs LIMIT 2";
    let q3 = "SELECT * FROM docs LIMIT 3";

    let _ = cache.parse(q1);
    let _ = cache.parse(q2);
    let _ = cache.parse(q1); // hit -> q1 gets the referenced bit
    let _ = cache.parse(q3); // miss -> eviction sweep: q2 evicted, q1 spared

    assert_eq!(cache.len(), 2);
    // q1 still hits (was spared), q2 should now miss.
    let hits_before = cache.stats().hits;
    let _ = cache.parse(q1);
    assert_eq!(cache.stats().hits, hits_before + 1, "q1 must survive");
}

#[test]
fn test_query_cache_hit_path_takes_no_write_lock() {
    // Issue #903: a hit must not need the write lock. We hold a read guard on
    // the cache and concurrently issue a hit from another thread; if the hit
    // tried to take a write lock it would deadlock against our read guard.
    use std::sync::Arc;
    use std::thread;

    let cache = Arc::new(QueryCache::new(10));
    let q = "SELECT * FROM docs LIMIT 1";
    let _ = cache.parse(q); // populate

    let held = cache.inner.read(); // hold a shared lock for the whole test

    let cache2 = Arc::clone(&cache);
    let handle = thread::spawn(move || cache2.parse(q).map(|_| ()));

    // If the hit path were write-locking, join() would block forever; the
    // test harness would hang. A successful join proves the hit is read-only.
    let res = handle
        .join()
        .expect("hit thread must finish without deadlock");
    assert!(res.is_ok());
    drop(held);
}

#[test]
fn test_query_cache_hit_returns_shared_arc() {
    // Issue #903: a hit returns Arc::clone of the stored AST, not a deep copy.
    let cache = QueryCache::new(10);
    let q = "SELECT * FROM docs LIMIT 1";

    let first = cache.parse(q).expect("parse");
    let second = cache.parse(q).expect("hit");

    assert!(
        Arc::ptr_eq(&first, &second),
        "hit must return the same Arc allocation (no deep clone)"
    );
    // The cache also retains its own reference, so strong count is >= 3.
    assert!(Arc::strong_count(&first) >= 3);
}

#[test]
#[expect(clippy::significant_drop_tightening)] // Reason: the guard under test is held to the assertion on purpose
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

    let inner = cache.inner.read();
    let mut uniq = std::collections::HashSet::new();
    for key in &inner.order {
        assert!(uniq.insert(key.clone()), "duplicate query in CLOCK ring");
    }
    assert_eq!(inner.order.len(), cache.len());
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
