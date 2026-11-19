#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::float_cmp
)]
//! Optimization Tests for `LruCache` (US-CORE-003-14)
//!
//! Validates `LruCache` performance characteristics.
//!
//! # Implementation Notes
//!
//! `LruList` (index-linked list + `HashMap`) makes get, insert, eviction and
//! remove O(1). The O(1) claim is guarded by two load-proof tests: a node-touch
//! count (deterministic) and a comparison of timing minima (black-box).

#![allow(clippy::similar_names)]
#![allow(clippy::doc_markdown)]

use std::time::Instant;

use super::LruCache;

/// Test that insert performance scales reasonably.
/// Eviction and insertion are O(1) list operations.
#[test]
#[ignore = "Performance test - run manually with --ignored, CI runners are too slow"]
fn test_insert_performance_scaling() {
    let sizes = [100, 1_000, 10_000];
    let mut times_per_op = vec![];

    for &size in &sizes {
        let cache: LruCache<u64, String> = LruCache::new(size);

        // Warm up
        for i in 0..size as u64 {
            cache.insert(i, format!("value_{i}"));
        }

        // Measure insert time (causes eviction each time)
        let ops = 1000;
        let start = Instant::now();
        for i in size as u64..(size as u64 + ops) {
            cache.insert(i, format!("new_value_{i}"));
        }
        let elapsed = start.elapsed();
        let time_per_op = elapsed.as_nanos() as f64 / ops as f64;
        times_per_op.push((size, time_per_op));
    }

    let ratio = times_per_op[2].1 / times_per_op[0].1;

    println!("Insert complexity test:");
    for (size, time) in &times_per_op {
        println!("  size={size}: {time:.1} ns/op");
    }
    println!("  Ratio 10K/100: {ratio:.1}x");

    // Loose bound kept from the former O(n) implementation (< 300x)
    // Note: CI machines may have higher variance
    assert!(ratio < 300.0, "Insert scaling too poor: ratio={ratio:.1}x");

    // Absolute performance: < 500µs per insert for 10K cache (CI machines are slower)
    assert!(
        times_per_op[2].1 < 500_000.0,
        "Insert too slow: {:.0} ns (target: < 500µs)",
        times_per_op[2].1
    );
}

/// Test get performance scaling.
/// Get includes move_to_back, an O(1) list relink.
#[test]
#[ignore = "Performance test - run manually with --ignored, CI runners are too slow"]
fn test_get_performance_scaling() {
    let sizes = [100, 1_000, 10_000];
    let mut times_per_op = vec![];

    for &size in &sizes {
        let cache: LruCache<u64, String> = LruCache::new(size);

        // Fill cache
        for i in 0..size as u64 {
            cache.insert(i, format!("value_{i}"));
        }

        // Measure get time
        let ops = 1000;
        let start = Instant::now();
        for i in 0..ops {
            let key = (i % size) as u64;
            let _ = cache.get(&key);
        }
        let elapsed = start.elapsed();
        let time_per_op = elapsed.as_nanos() as f64 / ops as f64;
        times_per_op.push((size, time_per_op));
    }

    let ratio = times_per_op[2].1 / times_per_op[0].1;

    println!("Get complexity test:");
    for (size, time) in &times_per_op {
        println!("  size={size}: {time:.1} ns/op");
    }
    println!("  Ratio 10K/100: {ratio:.1}x");

    // Absolute performance: < 500µs per get for 10K cache (CI machines are slower)
    assert!(
        times_per_op[2].1 < 500_000.0,
        "Get too slow: {:.0} ns (target: < 500µs)",
        times_per_op[2].1
    );
}

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

/// Test eviction performance scaling.
/// Eviction pops the list head in O(1).
#[test]
#[ignore = "Performance test - run manually with --ignored, CI runners are too slow"]
fn test_eviction_performance_scaling() {
    let sizes = [100, 1_000, 10_000];
    let mut times_per_eviction = vec![];

    for &size in &sizes {
        let cache: LruCache<u64, String> = LruCache::new(size);

        // Fill cache completely
        for i in 0..size as u64 {
            cache.insert(i, format!("value_{i}"));
        }

        // Measure time to insert (causing eviction)
        let ops = 500;
        let start = Instant::now();
        for i in 0..ops {
            let key = size as u64 + i as u64;
            cache.insert(key, format!("evict_{key}"));
        }
        let elapsed = start.elapsed();
        let time_per_op = elapsed.as_nanos() as f64 / ops as f64;
        times_per_eviction.push((size, time_per_op));
    }

    let ratio = times_per_eviction[2].1 / times_per_eviction[0].1;

    println!("Eviction complexity test:");
    for (size, time) in &times_per_eviction {
        println!("  size={size}: {time:.1} ns/eviction");
    }
    println!("  Ratio 10K/100: {ratio:.1}x");

    // Absolute performance: < 500µs per eviction for 10K cache (CI machines are slower)
    assert!(
        times_per_eviction[2].1 < 500_000.0,
        "Eviction too slow: {:.0} ns (target: < 500µs)",
        times_per_eviction[2].1
    );
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
