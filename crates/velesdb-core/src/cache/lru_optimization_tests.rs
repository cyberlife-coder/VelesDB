#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::float_cmp
)]
//! Optimization Tests for `LruCache` (US-CORE-003-14)
//!
//! Validates `LruCache` complexity-sensitive behavior. Timing assertions in
//! the suite are `#[ignore]`d (manual runs); per-size cost is measured by the
//! `LruCache-Baseline` group of `benches/concurrency_benchmark.rs`. The O(1)
//! claim itself is guarded without a clock: `LruList`'s slab counts its node
//! accesses in test builds, and that count must not grow with the size.
//!
//! # Implementation Notes
//!
//! `LruCache` sits on `LruList`, a hash map over a doubly linked slab, so
//! insert, get, remove and eviction are all O(1). The bounds below predate
//! it (they allowed the O(n) shifts of the `IndexMap` it replaced) and stay
//! as loose sanity checks.

#![allow(clippy::similar_names)]
#![allow(clippy::doc_markdown)]

use std::time::Instant;

use super::LruCache;

/// Test that insert performance scales reasonably.
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

    // Loose bound: O(1) operations should not scale with the size at all;
    // CI machines have high variance.
    assert!(ratio < 300.0, "Insert scaling too poor: ratio={ratio:.1}x");

    // Absolute performance: < 500µs per insert for 10K cache (CI machines are slower)
    assert!(
        times_per_op[2].1 < 500_000.0,
        "Insert too slow: {:.0} ns (target: < 500µs)",
        times_per_op[2].1
    );
}

/// Test get performance scaling.
/// Get includes the move to MRU.
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

/// Test eviction performance scaling.
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

/// `get` moves the key to the MRU end at every size the per-size benchmark
/// covers (`concurrency_benchmark`, `LruCache-Baseline/get_hit`), so the key
/// outlives every other resident through a full turnover of the cache.
///
/// This replaces a wall-clock ratio that claimed O(1) for move-to-back. It
/// timed `get(&0)` repeatedly, which after the first call hits the key already
/// at the MRU end, so it never exercised a move at all, and a ratio of two
/// sub-microsecond timings measures the runner. That `get` is O(1) holds by
/// construction (`LruList`'s hash map over a linked slab); its cost per size
/// is the benchmark's question.
#[test]
fn test_get_moves_key_to_mru_at_every_size() {
    for size in [100usize, 1_000, 10_000] {
        let cache: LruCache<u64, String> = LruCache::new(size);
        let size_u64 = size as u64;
        for i in 0..size_u64 {
            cache.insert(i, format!("value_{i}"));
        }

        assert!(cache.get(&0).is_some());
        // size - 1 new keys evict exactly the size - 1 keys older than 0.
        for i in size_u64..(2 * size_u64 - 1) {
            cache.insert(i, format!("value_{i}"));
        }

        assert_eq!(cache.stats().evictions, size_u64 - 1, "size={size}");
        assert!(cache.peek(&0).is_some(), "size={size}: MRU key evicted");
        assert!(cache.peek(&1).is_none(), "size={size}: LRU key survived");
    }
}

/// Every recency operation touches a constant number of list nodes.
///
/// The deterministic counterpart of the per-size benchmark: the count comes
/// from the node slab's accessors, not from a clock, so load cannot move it,
/// and an O(n) move-to-back, eviction or removal makes it grow with the size.
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
