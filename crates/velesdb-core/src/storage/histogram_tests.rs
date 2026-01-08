//! Tests for `histogram` module

use super::histogram::*;

#[test]
fn test_histogram_empty() {
    let h = LockFreeHistogram::new();
    assert!(h.is_empty());
    assert_eq!(h.count(), 0);
    assert_eq!(h.min(), 0);
    assert_eq!(h.max(), 0);
    assert_eq!(h.mean(), 0);
    assert_eq!(h.percentile(50), 0);
}

#[test]
fn test_histogram_is_empty() {
    let h = LockFreeHistogram::new();
    assert!(h.is_empty());

    h.record(100);
    assert!(!h.is_empty());

    h.reset();
    assert!(h.is_empty());
}

#[test]
fn test_histogram_single_value() {
    let h = LockFreeHistogram::new();
    h.record(100);

    assert_eq!(h.count(), 1);
    assert_eq!(h.min(), 100);
    assert_eq!(h.max(), 100);
    assert_eq!(h.mean(), 100);
}

#[test]
fn test_histogram_multiple_values() {
    let h = LockFreeHistogram::new();
    for i in 1..=100 {
        h.record(i);
    }

    assert_eq!(h.count(), 100);
    assert_eq!(h.min(), 1);
    assert_eq!(h.max(), 100);
    assert_eq!(h.mean(), 50); // (1+100)/2 = 50.5 → 50
}

#[test]
fn test_histogram_percentiles() {
    let h = LockFreeHistogram::new();
    // Record values that span multiple buckets
    for _ in 0..1000 {
        h.record(10); // ~10µs
    }
    for _ in 0..100 {
        h.record(1000); // ~1ms
    }
    for _ in 0..10 {
        h.record(100_000); // ~100ms
    }

    // P50 should be around 10µs (most values)
    let p50 = h.percentile(50);
    assert!(p50 < 100, "P50 should be low, got {p50}");

    // P99 should be higher
    let p99 = h.percentile(99);
    assert!(p99 > p50, "P99 ({p99}) should be > P50 ({p50})");
}

#[test]
fn test_histogram_reset() {
    let h = LockFreeHistogram::new();
    h.record(100);
    h.record(200);

    h.reset();

    assert_eq!(h.count(), 0);
    assert_eq!(h.min(), 0);
    assert_eq!(h.max(), 0);
}

#[test]
fn test_histogram_thread_safety() {
    use std::sync::Arc;
    use std::thread;

    let h = Arc::new(LockFreeHistogram::new());
    let num_threads = 4;
    let ops_per_thread = 10_000;

    let handles: Vec<_> = (0..num_threads)
        .map(|t| {
            let h = h.clone();
            thread::spawn(move || {
                for i in 0..ops_per_thread {
                    h.record(t * 1000 + i);
                }
            })
        })
        .collect();

    for handle in handles {
        handle.join().unwrap();
    }

    assert_eq!(h.count(), num_threads * ops_per_thread);
}

// Note: bucket_for is private, tested implicitly via percentile behavior
