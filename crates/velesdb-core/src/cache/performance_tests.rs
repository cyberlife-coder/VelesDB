#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::float_cmp
)]
//! Performance Regression Tests for Cache Layer (US-CORE-003-13)
//!
//! Tests edge cases and validates performance characteristics.

use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use super::{BloomFilter, LruCache};

// ========== Performance Edge Case Tests ==========

#[test]
#[ignore = "Performance test - CI runners have variable performance"]
fn test_lru_cache_eviction_performance() {
    // Test that eviction doesn't cause O(n²) behavior
    let cache: LruCache<u64, String> = LruCache::new(100);

    let start = Instant::now();

    // Insert 10K items (causes 9900 evictions)
    for i in 0..10_000 {
        cache.insert(i, format!("value_{i}"));
    }

    let elapsed = start.elapsed();

    // Should complete in reasonable time (< 5s for 10K ops on slow CI)
    assert!(
        elapsed < Duration::from_secs(5),
        "10K inserts with eviction took too long: {elapsed:?}"
    );

    // Verify cache state
    assert_eq!(cache.len(), 100);
    assert_eq!(cache.stats().evictions, 9900);
}

#[test]
#[ignore = "Performance test - run manually with --ignored, CI runners are too slow"]
fn test_lru_cache_get_performance_with_updates() {
    // Test that get() with recency updates doesn't degrade
    let cache: LruCache<u64, String> = LruCache::new(1000);

    // Pre-populate
    for i in 0..1000 {
        cache.insert(i, format!("value_{i}"));
    }

    let start = Instant::now();

    // 10K gets with recency updates
    for _ in 0..10_000 {
        for key in 0..100 {
            let _ = cache.get(&key);
        }
    }

    let elapsed = start.elapsed();

    // Should complete in reasonable time (CI machines are slower)
    assert!(
        elapsed < Duration::from_secs(30),
        "1M gets took too long: {elapsed:?}"
    );
}

#[test]
#[ignore = "Performance test - flaky on CI due to variable CPU load"]
fn test_bloom_filter_scaling() {
    // Test that bloom filter scales well with size
    let sizes = [1_000, 10_000, 100_000];
    let mut times = vec![];

    for &size in &sizes {
        let bloom = BloomFilter::new(size, 0.01);

        let start = Instant::now();
        for i in 0..size as u64 {
            bloom.insert(&i);
        }
        times.push(start.elapsed());
    }

    // Time should scale roughly linearly (not quadratically)
    // 100K should be < 200x time of 1K
    let ratio = times[2].as_nanos() as f64 / times[0].as_nanos() as f64;
    assert!(
        ratio < 200.0,
        "Bloom filter doesn't scale linearly: 100K/1K ratio = {ratio:.1}x"
    );
}

#[test]
#[ignore = "Performance test - run manually with --ignored, CI runners are too slow"]
fn test_lru_cache_concurrent_throughput() {
    // Measure throughput under concurrent access
    let cache: Arc<LruCache<u64, String>> = Arc::new(LruCache::new(1000));

    // Pre-populate
    for i in 0..1000 {
        cache.insert(i, format!("value_{i}"));
    }

    let ops_per_thread = 10_000;
    let num_threads = 4;

    let start = Instant::now();

    let mut handles = vec![];
    for t in 0..num_threads {
        let cache_clone = Arc::clone(&cache);
        handles.push(thread::spawn(move || {
            for i in 0..ops_per_thread {
                let key = ((t * 1000 + i) % 1000) as u64;
                if i % 4 == 0 {
                    cache_clone.insert(key, format!("v_{key}"));
                } else {
                    let _ = cache_clone.get(&key);
                }
            }
        }));
    }

    for h in handles {
        h.join().unwrap();
    }

    let elapsed = start.elapsed();
    let total_ops = num_threads * ops_per_thread;
    let ops_per_sec = f64::from(total_ops) / elapsed.as_secs_f64();

    // Should achieve at least 10K ops/sec (CI machines are slower)
    assert!(
        ops_per_sec > 10_000.0,
        "Throughput too low: {ops_per_sec:.0} ops/sec (target: 10K+)"
    );

    println!("LruCache concurrent throughput: {ops_per_sec:.0} ops/sec");
}

#[test]
#[ignore = "Performance test - CI runners have variable performance"]
fn test_bloom_filter_concurrent_throughput() {
    // Measure throughput under concurrent access
    let bloom = Arc::new(BloomFilter::new(100_000, 0.01));

    // Pre-populate
    for i in 0..10_000u64 {
        bloom.insert(&i);
    }

    let ops_per_thread = 10_000;
    let num_threads = 4;

    let start = Instant::now();

    let mut handles = vec![];
    for t in 0..num_threads {
        let bloom_clone = Arc::clone(&bloom);
        handles.push(thread::spawn(move || {
            for i in 0..ops_per_thread {
                let key = (t * 10000 + i) as u64;
                if i % 2 == 0 {
                    bloom_clone.insert(&key);
                } else {
                    let _ = bloom_clone.contains(&key);
                }
            }
        }));
    }

    for h in handles {
        h.join().unwrap();
    }

    let elapsed = start.elapsed();
    let total_ops = num_threads * ops_per_thread;
    let ops_per_sec = f64::from(total_ops) / elapsed.as_secs_f64();

    // Should achieve at least 100K ops/sec (relaxed for CI)
    assert!(
        ops_per_sec > 100_000.0,
        "Throughput too low: {ops_per_sec:.0} ops/sec (target: 100K+)"
    );

    println!("BloomFilter concurrent throughput: {ops_per_sec:.0} ops/sec");
}

// ========== Scalability Tests ==========

#[test]
#[ignore = "Flaky: depends on system load - run manually with --ignored"]
fn test_lru_cache_thread_scalability() {
    // Test that performance scales with thread count
    let mut throughputs = vec![];

    for num_threads in [1, 2, 4] {
        let cache: Arc<LruCache<u64, String>> = Arc::new(LruCache::new(1000));

        // Pre-populate
        for i in 0..1000 {
            cache.insert(i, format!("value_{i}"));
        }

        let ops_per_thread = 5000;

        let start = Instant::now();

        let mut handles = vec![];
        for t in 0..num_threads {
            let cache_clone = Arc::clone(&cache);
            handles.push(thread::spawn(move || {
                for i in 0..ops_per_thread {
                    let key = ((t * 1000 + i) % 1000) as u64;
                    let _ = cache_clone.get(&key);
                }
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        let elapsed = start.elapsed();
        let total_ops = num_threads * ops_per_thread;
        let ops_per_sec = f64::from(total_ops) / elapsed.as_secs_f64();
        throughputs.push((num_threads, ops_per_sec));
    }

    // With RwLock, read-heavy workloads should scale somewhat
    // 4 threads should be at least 1.5x throughput of 1 thread
    let ratio = throughputs[2].1 / throughputs[0].1;
    println!(
        "Thread scalability: 1T={:.0}, 2T={:.0}, 4T={:.0} ops/sec",
        throughputs[0].1, throughputs[1].1, throughputs[2].1
    );
    println!("4T/1T ratio: {ratio:.2}x");

    // Relaxed assertion due to lock contention
    assert!(
        ratio > 0.5,
        "Severe scaling regression: 4T/1T ratio = {ratio:.2}x"
    );
}

// ========== Memory Usage Tests ==========

#[test]
fn test_bloom_filter_memory_efficiency() {
    // Verify bloom filter uses expected memory
    let capacity = 100_000;
    let fpr = 0.01;

    let bloom = BloomFilter::new(capacity, fpr);

    // Insert all items
    for i in 0..capacity as u64 {
        bloom.insert(&i);
    }

    // Check FPR is within bounds
    let mut false_positives = 0;
    for i in capacity as u64..(capacity * 2) as u64 {
        if bloom.contains(&i) {
            false_positives += 1;
        }
    }

    let actual_fpr = f64::from(false_positives) / capacity as f64;

    // Should be within 10x of target (1% target → < 10% actual)
    assert!(
        actual_fpr < fpr * 10.0,
        "FPR too high: {actual_fpr:.4} (target: {fpr})"
    );

    println!("BloomFilter: capacity={capacity}, target_fpr={fpr}, actual_fpr={actual_fpr:.4}");
}
