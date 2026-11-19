//! Concurrency Benchmarks for EPIC-CORE-003
//!
//! Measures performance scaling and contention under concurrent access.
//! Run with: cargo bench --bench concurrency_benchmark

#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::doc_markdown)]

use criterion::measurement::WallTime;
use criterion::{
    black_box, criterion_group, criterion_main, BenchmarkGroup, BenchmarkId, Criterion, Throughput,
};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;

use velesdb_core::cache::{BloomFilter, LockFreeLruCache, LruCache};

// ========== Single-Thread Baseline Benchmarks ==========

fn bench_lru_cache_baseline(c: &mut Criterion) {
    let mut group = c.benchmark_group("LruCache-Baseline");

    for size in [100, 1_000, 10_000] {
        group.throughput(Throughput::Elements(1000));

        group.bench_with_input(BenchmarkId::new("insert", size), &size, |b, &size| {
            let cache: LruCache<u64, String> = LruCache::new(size);
            let mut i = 0u64;
            b.iter(|| {
                cache.insert(i, format!("value_{i}"));
                i = (i + 1) % (size as u64 * 2);
            });
        });

        group.bench_with_input(BenchmarkId::new("get_hit", size), &size, |b, &size| {
            let cache: LruCache<u64, String> = LruCache::new(size);
            // Pre-populate
            for i in 0..size as u64 {
                cache.insert(i, format!("value_{i}"));
            }
            let mut i = 0u64;
            b.iter(|| {
                let _ = black_box(cache.get(&i));
                i = (i + 1) % (size as u64);
            });
        });

        group.bench_with_input(BenchmarkId::new("get_miss", size), &size, |b, &size| {
            let cache: LruCache<u64, String> = LruCache::new(size);
            // Pre-populate with different keys
            for i in 0..size as u64 {
                cache.insert(i, format!("value_{i}"));
            }
            let miss_key = size as u64 + 1000;
            b.iter(|| {
                let _ = black_box(cache.get(&miss_key));
            });
        });
    }

    group.finish();
}

fn bench_bloom_filter_baseline(c: &mut Criterion) {
    let mut group = c.benchmark_group("BloomFilter-Baseline");

    for capacity in [1_000, 10_000, 100_000] {
        group.throughput(Throughput::Elements(1000));

        group.bench_with_input(
            BenchmarkId::new("insert", capacity),
            &capacity,
            |b, &cap| {
                let bloom = BloomFilter::new(cap, 0.01);
                let mut i = 0u64;
                b.iter(|| {
                    bloom.insert(&i);
                    i = i.wrapping_add(1);
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("contains_positive", capacity),
            &capacity,
            |b, &cap| {
                let bloom = BloomFilter::new(cap, 0.01);
                // Pre-populate
                for i in 0..cap as u64 {
                    bloom.insert(&i);
                }
                let mut i = 0u64;
                b.iter(|| {
                    let _ = black_box(bloom.contains(&i));
                    i = (i + 1) % (cap as u64);
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("contains_negative", capacity),
            &capacity,
            |b, &cap| {
                let bloom = BloomFilter::new(cap, 0.01);
                // Pre-populate
                for i in 0..cap as u64 {
                    bloom.insert(&i);
                }
                // Query with keys definitely not present
                let mut i = cap as u64 * 2;
                b.iter(|| {
                    let _ = black_box(bloom.contains(&i));
                    i = i.wrapping_add(1);
                });
            },
        );
    }

    group.finish();
}

// ========== Multi-Thread Scaling Benchmarks ==========

#[allow(clippy::too_many_lines)] // Reason: benchmark harness with multiple group configurations; splitting would hurt readability.
fn bench_lru_cache_concurrent(c: &mut Criterion) {
    let mut group = c.benchmark_group("LruCache-Concurrent");

    for num_threads in [1, 2, 4, 8] {
        let ops_per_thread = 1000;
        group.throughput(Throughput::Elements((num_threads * ops_per_thread) as u64));

        group.bench_with_input(
            BenchmarkId::new("mixed_read_write", num_threads),
            &num_threads,
            |b, &n_threads| {
                b.iter(|| {
                    let cache: Arc<LruCache<u64, String>> = Arc::new(LruCache::new(1000));

                    // Pre-populate
                    for i in 0..500 {
                        cache.insert(i, format!("value_{i}"));
                    }

                    let mut handles = vec![];

                    for t in 0..n_threads {
                        let cache_clone = Arc::clone(&cache);
                        handles.push(thread::spawn(move || {
                            for i in 0..ops_per_thread {
                                let key = (t * 1000 + i) as u64;
                                if i % 4 == 0 {
                                    // 25% writes
                                    cache_clone.insert(key, format!("v_{key}"));
                                } else {
                                    // 75% reads
                                    let _ = cache_clone.get(&(i as u64 % 500));
                                }
                            }
                        }));
                    }

                    for h in handles {
                        h.join().unwrap();
                    }
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("read_heavy", num_threads),
            &num_threads,
            |b, &n_threads| {
                b.iter(|| {
                    let cache: Arc<LruCache<u64, String>> = Arc::new(LruCache::new(1000));

                    // Pre-populate
                    for i in 0..1000 {
                        cache.insert(i, format!("value_{i}"));
                    }

                    let mut handles = vec![];

                    for _ in 0..n_threads {
                        let cache_clone = Arc::clone(&cache);
                        handles.push(thread::spawn(move || {
                            for i in 0..ops_per_thread {
                                let _ = cache_clone.get(&(i as u64 % 1000));
                            }
                        }));
                    }

                    for h in handles {
                        h.join().unwrap();
                    }
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("write_heavy", num_threads),
            &num_threads,
            |b, &n_threads| {
                b.iter(|| {
                    let cache: Arc<LruCache<u64, String>> = Arc::new(LruCache::new(1000));

                    let mut handles = vec![];

                    for t in 0..n_threads {
                        let cache_clone = Arc::clone(&cache);
                        handles.push(thread::spawn(move || {
                            for i in 0..ops_per_thread {
                                let key = (t * 10000 + i) as u64;
                                cache_clone.insert(key, format!("v_{key}"));
                            }
                        }));
                    }

                    for h in handles {
                        h.join().unwrap();
                    }
                });
            },
        );
    }

    group.finish();
}

fn bench_bloom_filter_concurrent(c: &mut Criterion) {
    let mut group = c.benchmark_group("BloomFilter-Concurrent");

    for num_threads in [1, 2, 4, 8] {
        let ops_per_thread = 1000;
        group.throughput(Throughput::Elements((num_threads * ops_per_thread) as u64));

        group.bench_with_input(
            BenchmarkId::new("insert_concurrent", num_threads),
            &num_threads,
            |b, &n_threads| {
                b.iter(|| {
                    let bloom = Arc::new(BloomFilter::new(100_000, 0.01));

                    let mut handles = vec![];

                    for t in 0..n_threads {
                        let bloom_clone = Arc::clone(&bloom);
                        handles.push(thread::spawn(move || {
                            for i in 0..ops_per_thread {
                                let key = (t * 10000 + i) as u64;
                                bloom_clone.insert(&key);
                            }
                        }));
                    }

                    for h in handles {
                        h.join().unwrap();
                    }
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("contains_concurrent", num_threads),
            &num_threads,
            |b, &n_threads| {
                b.iter(|| {
                    let bloom = Arc::new(BloomFilter::new(100_000, 0.01));

                    // Pre-populate
                    for i in 0..10_000u64 {
                        bloom.insert(&i);
                    }

                    let mut handles = vec![];

                    for _ in 0..n_threads {
                        let bloom_clone = Arc::clone(&bloom);
                        handles.push(thread::spawn(move || {
                            for i in 0..ops_per_thread {
                                let _ = bloom_clone.contains(&(i as u64));
                            }
                        }));
                    }

                    for h in handles {
                        h.join().unwrap();
                    }
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("mixed_insert_contains", num_threads),
            &num_threads,
            |b, &n_threads| {
                b.iter(|| {
                    let bloom = Arc::new(BloomFilter::new(100_000, 0.01));

                    let mut handles = vec![];

                    for t in 0..n_threads {
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
                });
            },
        );
    }

    group.finish();
}

// ========== Contention Measurement ==========

fn bench_contention(c: &mut Criterion) {
    let mut group = c.benchmark_group("Contention");

    // High contention: all threads access same keys
    group.bench_function("lru_high_contention_8_threads", |b| {
        b.iter(|| {
            let cache: Arc<LruCache<u64, String>> = Arc::new(LruCache::new(100));

            // Pre-populate with just 10 keys (high contention)
            for i in 0..10 {
                cache.insert(i, format!("value_{i}"));
            }

            let mut handles = vec![];

            for _ in 0..8 {
                let cache_clone = Arc::clone(&cache);
                handles.push(thread::spawn(move || {
                    for i in 0..100 {
                        let key = (i % 10) as u64;
                        let _ = cache_clone.get(&key);
                        cache_clone.insert(key, format!("updated_{key}"));
                    }
                }));
            }

            for h in handles {
                h.join().unwrap();
            }
        });
    });

    // Low contention: threads access different keys
    group.bench_function("lru_low_contention_8_threads", |b| {
        b.iter(|| {
            let cache: Arc<LruCache<u64, String>> = Arc::new(LruCache::new(10_000));

            // Pre-populate with many keys
            for i in 0..8000 {
                cache.insert(i, format!("value_{i}"));
            }

            let mut handles = vec![];

            for t in 0..8 {
                let cache_clone = Arc::clone(&cache);
                handles.push(thread::spawn(move || {
                    for i in 0..100 {
                        let key = (t * 1000 + i) as u64;
                        let _ = cache_clone.get(&key);
                        cache_clone.insert(key + 10000, format!("new_{key}"));
                    }
                }));
            }

            for h in handles {
                h.join().unwrap();
            }
        });
    });

    group.finish();
}

/// The compiled plan cache's sizes (`Database::open`): L1 1,000, L2 10,000.
const PLAN_CACHE_L1: usize = 1_000;
const PLAN_CACHE_L2: usize = 10_000;
/// Keys the readers cycle through: half of L1, so an LRU L1 keeps them all
/// resident while fresh inserts take the other half.
const HOT_KEYS: u64 = PLAN_CACHE_L1 as u64 / 2;
const OPS_PER_THREAD: u64 = 10_000;

/// `LockFreeLruCache` at 1 to 8 threads, at the plan cache's sizes.
///
/// Each case builds and fills its cache once, outside the measured loop, so
/// an iteration times only the threads' operations (and their spawn,
/// amortised over `OPS_PER_THREAD`). Compare a thread count's throughput
/// with the 1-thread line to read the scaling.
fn bench_lockfree_lru_cache_concurrent(c: &mut Criterion) {
    let mut group = c.benchmark_group("LockFreeLruCache-Concurrent");
    for num_threads in [1_u64, 2, 4, 8] {
        group.throughput(Throughput::Elements(num_threads * OPS_PER_THREAD));
        bench_l1_hit_read(&mut group, num_threads);
        bench_read_mostly_with_eviction(&mut group, num_threads);
    }
    group.finish();
}

/// Every read hits L1: the lock-free path the plan cache serves from.
fn bench_l1_hit_read(group: &mut BenchmarkGroup<'_, WallTime>, num_threads: u64) {
    let cache = filled_lockfree_cache();
    group.bench_function(BenchmarkId::new("l1_hit_read", num_threads), |b| {
        b.iter(|| {
            on_threads(num_threads, |thread, op| {
                black_box(cache.get(&hot_key(thread, op)));
            });
        });
    });
}

/// 90% reads of the hot keys, 10% inserts of fresh keys. Every insert takes
/// L1 over capacity, so this times the eviction path beside the reads. An
/// LRU L1 evicts the fresh keys and keeps the reads L1 hits; an eviction
/// that ignores recency turns some of them into L2 hits, measured here too.
fn bench_read_mostly_with_eviction(group: &mut BenchmarkGroup<'_, WallTime>, num_threads: u64) {
    let cache = filled_lockfree_cache();
    let next_fresh_key = AtomicU64::new(PLAN_CACHE_L2 as u64);
    group.bench_function(
        BenchmarkId::new("read_mostly_with_eviction", num_threads),
        |b| {
            b.iter(|| {
                on_threads(num_threads, |thread, op| {
                    if op % 10 == 0 {
                        let key = next_fresh_key.fetch_add(1, Ordering::Relaxed);
                        cache.insert(key, key);
                    } else {
                        black_box(cache.get(&hot_key(thread, op)));
                    }
                });
            });
        },
    );
}

/// Runs `op(thread, op_index)` `OPS_PER_THREAD` times on each of
/// `num_threads` scoped threads.
fn on_threads(num_threads: u64, op: impl Fn(u64, u64) + Sync) {
    thread::scope(|scope| {
        for thread in 0..num_threads {
            let op = &op;
            scope.spawn(move || (0..OPS_PER_THREAD).for_each(|i| op(thread, i)));
        }
    });
}

/// The hot key a thread reads at its `op`-th operation; threads start at
/// different offsets so they do not read in lockstep.
fn hot_key(thread: u64, op: u64) -> u64 {
    (op + thread * 100) % HOT_KEYS
}

/// A `LockFreeLruCache` at the plan cache's sizes with L1 full of keys
/// `0..PLAN_CACHE_L1`.
fn filled_lockfree_cache() -> LockFreeLruCache<u64, u64> {
    let cache = LockFreeLruCache::new(PLAN_CACHE_L1, PLAN_CACHE_L2);
    for key in 0..PLAN_CACHE_L1 as u64 {
        cache.insert(key, key);
    }
    cache
}

criterion_group!(
    benches,
    bench_lru_cache_baseline,
    bench_bloom_filter_baseline,
    bench_lru_cache_concurrent,
    bench_lockfree_lru_cache_concurrent,
    bench_bloom_filter_concurrent,
    bench_contention,
);

criterion_main!(benches);
