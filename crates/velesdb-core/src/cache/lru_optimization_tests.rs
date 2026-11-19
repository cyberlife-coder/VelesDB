#![allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
//! Complexity tests for `LruCache` (US-CORE-003-14)
//!
//! `LruList` (index-linked list + `HashMap`) makes get, insert, eviction and
//! remove O(1). Every guard here is load-proof: node-touch counts are
//! deterministic, and the one timing test compares interleaved minima.

use std::time::Instant;

use super::LruCache;

/// Peek skips the recency update, so it does strictly less work than get.
///
/// Measured in list nodes touched rather than wall-clock: since get became
/// O(1) the two differ by ~1.3x in time, a margin no clock comparison keeps
/// under load.
#[test]
fn test_peek_does_less_work_than_get() {
    let cache: LruCache<u64, String> = LruCache::new(1000);
    for i in 0..1000 {
        cache.insert(i, format!("value_{i}"));
    }

    let peek = cache.nodes_touched_by(|c| assert!(c.peek(&0).is_some()));
    let get = cache.nodes_touched_by(|c| assert!(c.get(&0).is_some()));

    assert!(peek < get, "peek touched {peek} nodes, get {get}");
}

/// Per-op time of the fastest batch of `get` calls on the LRU (front) key.
///
/// Reading keys in insertion order always hits the current LRU entry, the
/// worst position for a move-to-back that shifts the entries behind it.
fn min_ns_per_lru_get(cache: &LruCache<u64, u64>, size: u64, next_key: &mut u64) -> f64 {
    const BATCH: u32 = 32;
    let start = Instant::now();
    for _ in 0..BATCH {
        let _ = cache.get(&(*next_key % size));
        *next_key += 1;
    }
    start.elapsed().as_nanos() as f64 / f64::from(BATCH)
}

/// Move-to-back must not scale with the cache size (black-box guard).
///
/// Load-robust by construction: sizes are sampled interleaved, so both see the
/// same machine conditions, and only the fastest batch per size is compared —
/// preemption and contention can only slow a batch down, never speed it up.
/// An O(n) move-to-back costs ~100x more at 10 000 than at 100 entries, an
/// O(1) one stays within cache-locality noise (~1-2x), hence the 10x bound.
#[test]
fn test_move_to_back_time_is_size_independent() {
    const SMALL: u64 = 100;
    const LARGE: u64 = 10_000;
    const REPETITIONS: usize = 200;

    let small: LruCache<u64, u64> = LruCache::new(SMALL as usize);
    let large: LruCache<u64, u64> = LruCache::new(LARGE as usize);
    (0..SMALL).for_each(|i| small.insert(i, i));
    (0..LARGE).for_each(|i| large.insert(i, i));

    let (mut next_small, mut next_large) = (0, 0);
    let (mut best_small, mut best_large) = (f64::MAX, f64::MAX);
    for _ in 0..REPETITIONS {
        best_small = best_small.min(min_ns_per_lru_get(&small, SMALL, &mut next_small));
        best_large = best_large.min(min_ns_per_lru_get(&large, LARGE, &mut next_large));
    }

    let ratio = best_large / best_small.max(1.0);
    println!("move-to-back min ns/op: size={SMALL}: {best_small:.1}, size={LARGE}: {best_large:.1}, ratio {ratio:.1}x");
    assert!(
        ratio < 10.0,
        "move-to-back scales with size: ratio={ratio:.1}x (O(1) expected, bound 10x)"
    );
}

/// Every recency operation touches a constant number of list nodes (structural guard).
///
/// Deterministic, so immune to load: the count comes from the node accessors of
/// the intrusive list, not from a clock.
#[test]
fn test_recency_operations_touch_constant_nodes() {
    let touches_at = |size: u64| {
        let cache: LruCache<u64, u64> = LruCache::new(size as usize);
        (0..size).for_each(|i| cache.insert(i, i));
        [
            cache.nodes_touched_by(|c| {
                let _ = c.get(&0); // LRU front -> MRU back
            }),
            cache.nodes_touched_by(|c| c.insert(size, size)), // evicts LRU
            cache.nodes_touched_by(|c| c.insert(size / 2, 0)), // update + promote
            cache.nodes_touched_by(|c| c.remove(&(size / 3))), // middle node
        ]
    };

    let small = touches_at(100);
    let large = touches_at(10_000);

    assert!(
        small.iter().all(|&t| t > 0),
        "node accessors not instrumented: {small:?}"
    );
    assert_eq!(small, large, "node touches grow with cache size");
}
