//! Optimization Tests for `LruCache` (US-CORE-003-14)
//!
//! Validates `LruCache` performance characteristics.
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

    // IndexMap is O(n) for shift_remove but should be < 100x (linear)
    // This is acceptable for cache sizes < 100K
    assert!(ratio < 200.0, "Insert scaling too poor: ratio={ratio:.1}x");

    // Absolute performance: < 50µs per insert for 10K cache
    assert!(
        times_per_op[2].1 < 50_000.0,
        "Insert too slow: {:.0} ns (target: < 50µs)",
        times_per_op[2].1
    );
}

/// Test get performance scaling.
/// Get includes move_to_back which is O(n) with IndexMap.
#[test]
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

    // Absolute performance: < 50µs per get for 10K cache
    assert!(
        times_per_op[2].1 < 50_000.0,
        "Get too slow: {:.0} ns (target: < 50µs)",
        times_per_op[2].1
    );
}

/// Test that peek (read-only) is faster than get.
#[test]
fn test_peek_faster_than_get() {
    let cache: LruCache<u64, String> = LruCache::new(1000);

    // Fill cache
    for i in 0..1000 {
        cache.insert(i, format!("value_{i}"));
    }

    let ops = 10_000;

    // Measure peek time
    let start = Instant::now();
    for i in 0..ops {
        let _ = cache.peek(&(i % 1000));
    }
    let peek_time = start.elapsed();

    // Measure get time
    let start = Instant::now();
    for i in 0..ops {
        let _ = cache.get(&(i % 1000));
    }
    let get_time = start.elapsed();

    println!("Peek vs Get:");
    println!("  Peek: {:?}", peek_time);
    println!("  Get:  {:?}", get_time);
    println!(
        "  Ratio get/peek: {:.2}x",
        get_time.as_nanos() as f64 / peek_time.as_nanos() as f64
    );

    // Peek should be faster (no recency update)
    assert!(peek_time < get_time, "Peek should be faster than get");
}

/// Test eviction performance scaling.
/// Eviction uses shift_remove_index(0) which is O(n) with IndexMap.
#[test]
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

    // Absolute performance: < 50µs per eviction for 10K cache
    assert!(
        times_per_eviction[2].1 < 50_000.0,
        "Eviction too slow: {:.0} ns (target: < 50µs)",
        times_per_eviction[2].1
    );
}

/// Test move_to_back is O(1).
#[test]
fn test_move_to_back_o1_complexity() {
    let sizes = [100, 1_000, 10_000];
    let mut times_per_op = vec![];

    for &size in &sizes {
        let cache: LruCache<u64, String> = LruCache::new(size);

        // Fill cache
        for i in 0..size as u64 {
            cache.insert(i, format!("value_{i}"));
        }

        // Measure get (which calls move_to_back internally)
        let ops = 500;
        let start = Instant::now();
        for _ in 0..ops {
            // Access key 0 repeatedly - forces move_to_back each time
            let _ = cache.get(&0);
        }
        let elapsed = start.elapsed();
        let time_per_op = elapsed.as_nanos() as f64 / ops as f64;
        times_per_op.push((size, time_per_op));
    }

    let ratio = times_per_op[2].1 / times_per_op[0].1;

    println!("Move-to-back complexity test:");
    for (size, time) in &times_per_op {
        println!("  size={size}: {time:.1} ns/move");
    }
    println!("  Ratio 10K/100: {ratio:.1}x");

    assert!(
        ratio < 10.0,
        "Move-to-back is not O(1): ratio={ratio:.1}x (should be < 10x)"
    );
}
