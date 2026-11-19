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
//! `LruCache-Baseline` group of `benches/concurrency_benchmark.rs`.
//!
//! # Implementation Notes
//!
//! `IndexMap` provides O(1) lookup but O(n) for `shift_remove` (preserves order).
//! This is still faster than `VecDeque::retain` which was O(n) with higher constant.
//! For true O(1) LRU, a custom doubly-linked list + `HashMap` would be needed.

#![allow(clippy::similar_names)]
#![allow(clippy::doc_markdown)]

use std::time::Instant;

use super::LruCache;

/// Test that insert performance scales reasonably.
/// IndexMap shift_remove is O(n) but with low constant.
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

    // IndexMap is O(n) for shift_remove but should be < 300x (linear)
    // This is acceptable for cache sizes < 100K
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
/// Get includes move_to_back which is O(n) with IndexMap.
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

/// `peek` is the read-only lookup: unlike `get` it must not touch recency.
/// (Its speed relative to `get` is a benchmark question, not a test one.)
#[test]
fn test_peek_does_not_update_recency() {
    let cache: LruCache<u64, String> = LruCache::new(3);
    for i in 1..=3 {
        cache.insert(i, format!("value_{i}"));
    }

    assert_eq!(cache.peek(&1), Some("value_1".to_string()));
    cache.insert(4, "value_4".to_string());

    assert_eq!(
        cache.peek(&1),
        None,
        "peeked key stayed LRU and was evicted"
    );
    assert_eq!(cache.peek(&2), Some("value_2".to_string()));
}

/// Test eviction performance scaling.
/// Eviction uses shift_remove_index(0) which is O(n) with IndexMap.
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
/// at the back — the one position where `shift_remove` is O(1) — and in
/// general `get` is O(n), as the `lru` module documents.
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
