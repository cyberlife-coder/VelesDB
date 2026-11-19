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
/// O(1) the two differ in time by too thin a margin for a clock comparison to
/// hold under load.
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

const SMALL: u64 = 100;
const LARGE: u64 = 10_000;
/// An O(n) operation scales with the `LARGE / SMALL` size gap, well past this
/// bound; an O(1) one stays within cache-locality noise, well below it.
const MAX_RATIO: f64 = 10.0;

/// A full cache holding keys `0..size`, oldest first.
fn full_cache(size: u64) -> LruCache<u64, u64> {
    let cache = LruCache::new(size as usize);
    (0..size).for_each(|i| cache.insert(i, i));
    cache
}

/// Fastest per-op batch time at `LARGE` divided by the one at `SMALL`.
///
/// `op(cache, size, n)` runs the n-th operation on a full cache of `size`.
/// Load-robust by construction: the sizes are sampled interleaved, so both see
/// the same machine conditions, and only the fastest batch per size counts —
/// preemption and contention can only slow a batch down, never speed it up.
/// The minimum also leaves out the hash index's occasional in-place tombstone
/// rehash, which is amortized O(1) and not what these tests guard.
fn fastest_batch_ratio(label: &str, op: impl Fn(&LruCache<u64, u64>, u64, u64)) -> f64 {
    const BATCH: u64 = 32;
    const REPETITIONS: usize = 200;
    let sizes = [SMALL, LARGE];
    let caches = sizes.map(full_cache);
    let mut next = [0_u64; 2];
    let mut best = [f64::MAX; 2];
    for _ in 0..REPETITIONS {
        for i in 0..2 {
            let start = Instant::now();
            for _ in 0..BATCH {
                op(&caches[i], sizes[i], next[i]);
                next[i] += 1;
            }
            best[i] = best[i].min(start.elapsed().as_nanos() as f64 / BATCH as f64);
        }
    }
    let ratio = best[1] / best[0].max(1.0);
    println!(
        "{label} min ns/op: size={SMALL}: {:.1}, size={LARGE}: {:.1}, ratio {ratio:.1}x",
        best[0], best[1]
    );
    ratio
}

/// Move-to-back must not scale with the cache size (black-box guard).
///
/// Reading keys in insertion order always hits the current LRU entry, the
/// worst position for a move-to-back that shifts the entries behind it.
#[test]
fn test_move_to_back_time_is_size_independent() {
    let ratio = fastest_batch_ratio("move-to-back", |cache, size, n| {
        let _ = cache.get(&(n % size));
    });
    assert!(
        ratio < MAX_RATIO,
        "move-to-back scales with size: ratio={ratio:.1}x (O(1) expected)"
    );
}

/// Eviction must not scale with the cache size (black-box guard).
///
/// Each insert brings a key never seen before into a full cache, so every one
/// evicts the current LRU entry.
#[test]
fn test_eviction_time_is_size_independent() {
    let ratio = fastest_batch_ratio("eviction", |cache, size, n| cache.insert(size + n, n));
    assert!(
        ratio < MAX_RATIO,
        "eviction scales with size: ratio={ratio:.1}x (O(1) expected)"
    );
}

/// Every recency operation touches a constant number of list nodes (structural guard).
///
/// Deterministic, so immune to load: the count comes from the node accessors of
/// the intrusive list, not from a clock.
#[test]
fn test_recency_operations_touch_constant_nodes() {
    let touches_at = |size: u64| {
        let cache = full_cache(size);
        [
            cache.nodes_touched_by(|c| {
                let _ = c.get(&0); // LRU front -> MRU back
            }),
            cache.nodes_touched_by(|c| c.insert(size, size)), // evicts LRU
            cache.nodes_touched_by(|c| c.insert(size / 2, 0)), // update + promote
            cache.nodes_touched_by(|c| c.remove(&(size / 3))), // middle node
        ]
    };

    let small = touches_at(SMALL);
    let large = touches_at(LARGE);

    assert!(
        small.iter().all(|&t| t > 0),
        "node accessors not instrumented: {small:?}"
    );
    assert_eq!(small, large, "node touches grow with cache size");
}
