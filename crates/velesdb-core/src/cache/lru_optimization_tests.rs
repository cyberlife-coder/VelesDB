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
//! `LruCache` sits on `LruList`, a hash map over a doubly linked slab, so
//! insert, get, remove and eviction are all O(1). The O(1) claim is guarded
//! by load-proof tests: a node-touch count (deterministic) and comparisons of
//! interleaved timing minima (black-box). The `#[ignore]`d bounds predate it
//! (they allowed the O(n) shifts of the `IndexMap` it replaced) and stay as
//! loose manual sanity checks.

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
/// worst position for a move-to-back that shifts the entries behind it. (The
/// test this replaces re-read key 0, which is the MRU after its first read, so
/// it timed a no-op; and one wall-clock ratio broke under a loaded suite.)
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
/// Deterministic, so immune to load: the count comes from the node slab's
/// accessors, not from a clock.
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
