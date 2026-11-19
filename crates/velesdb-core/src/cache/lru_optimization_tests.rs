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
