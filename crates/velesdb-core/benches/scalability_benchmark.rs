//! Scalability Benchmarks for `VelesDB` HNSW Index
//!
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
//!
//! This benchmark is NOT included in CI due to long execution times.
//! Run manually with: `cargo bench --bench scalability_benchmark`
//!
//! Tests:
//! - Scalability: 100k, 500k, 1M vectors
//! - Latency percentiles: p50, p95, p99
//! - Memory usage: peak RSS
//! - Concurrent queries: multi-threaded search

use criterion::{
    black_box, criterion_group, criterion_main, BenchmarkId, Criterion, SamplingMode, Throughput,
};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use velesdb_core::{DistanceMetric, HnswIndex, VectorIndex};

// ============================================================================
// Configuration
// ============================================================================

/// Vector dimension (typical for embeddings like `OpenAI`, Cohere, etc.)
const DIMENSION: usize = 768;

/// Dataset sizes to benchmark
const DATASET_SIZES: &[usize] = &[100_000, 500_000, 1_000_000];

/// Number of concurrent threads for throughput tests
const THREAD_COUNTS: &[usize] = &[1, 2, 4, 8];

/// Number of queries per thread in concurrent tests
const QUERIES_PER_THREAD: usize = 100;

// ============================================================================
// Vector Generation
// ============================================================================

/// Generates a deterministic pseudo-random vector for benchmarking.
/// Uses a simple hash-based approach for reproducibility.
#[allow(clippy::cast_precision_loss)]
fn generate_vector(dim: usize, seed: u64) -> Vec<f32> {
    (0..dim)
        .map(|i| {
            let x = seed.wrapping_mul(2_654_435_761) ^ (i as u64).wrapping_mul(2_246_822_519);
            let normalized = (x as f32) / (u64::MAX as f32);
            normalized * 2.0 - 1.0 // Range [-1, 1]
        })
        .collect()
}

/// Normalizes a vector to unit length (for cosine similarity).
fn normalize(v: &mut [f32]) {
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in v.iter_mut() {
            *x /= norm;
        }
    }
}

/// Generates a normalized vector suitable for cosine similarity.
fn generate_normalized_vector(dim: usize, seed: u64) -> Vec<f32> {
    let mut v = generate_vector(dim, seed);
    normalize(&mut v);
    v
}

// ============================================================================
// Memory Measurement (Cross-platform)
// ============================================================================

/// Returns current process memory usage in bytes (best effort).
#[cfg(target_os = "windows")]
fn get_memory_usage() -> usize {
    use std::mem::MaybeUninit;

    #[repr(C)]
    struct ProcessMemoryCounters {
        cb: u32,
        page_fault_count: u32,
        peak_working_set_size: usize,
        working_set_size: usize,
        quota_peak_paged_pool_usage: usize,
        quota_paged_pool_usage: usize,
        quota_peak_non_paged_pool_usage: usize,
        quota_non_paged_pool_usage: usize,
        pagefile_usage: usize,
        peak_pagefile_usage: usize,
    }

    #[link(name = "psapi")]
    extern "system" {
        fn GetProcessMemoryInfo(
            process: isize,
            ppsmem_counters: *mut ProcessMemoryCounters,
            cb: u32,
        ) -> i32;
        fn GetCurrentProcess() -> isize;
    }

    unsafe {
        let mut pmc = MaybeUninit::<ProcessMemoryCounters>::zeroed().assume_init();
        pmc.cb = std::mem::size_of::<ProcessMemoryCounters>() as u32;
        #[allow(clippy::borrow_as_ptr)]
        if GetProcessMemoryInfo(
            GetCurrentProcess(),
            &mut pmc,
            std::mem::size_of::<ProcessMemoryCounters>() as u32,
        ) != 0
        {
            pmc.working_set_size
        } else {
            0
        }
    }
}

#[cfg(target_os = "linux")]
fn get_memory_usage() -> usize {
    use std::fs;
    fs::read_to_string("/proc/self/statm")
        .ok()
        .and_then(|s| {
            let parts: Vec<&str> = s.split_whitespace().collect();
            parts.get(1).and_then(|p| p.parse::<usize>().ok())
        })
        .map_or(0, |pages| pages * 4096) // Page size typically 4KB
}

#[cfg(target_os = "macos")]
fn get_memory_usage() -> usize {
    // macOS: use rusage
    use std::mem::MaybeUninit;

    #[repr(C)]
    struct Rusage {
        ru_utime: [i64; 2],
        ru_stime: [i64; 2],
        ru_maxrss: i64,
        // ... other fields we don't need
        _padding: [i64; 13],
    }

    extern "C" {
        fn getrusage(who: i32, usage: *mut Rusage) -> i32;
    }

    unsafe {
        let mut usage = MaybeUninit::<Rusage>::zeroed().assume_init();
        if getrusage(0, &mut usage) == 0 {
            usage.ru_maxrss as usize
        } else {
            0
        }
    }
}

#[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
fn get_memory_usage() -> usize {
    0 // Unsupported platform
}

fn format_bytes(bytes: usize) -> String {
    if bytes >= 1024 * 1024 * 1024 {
        format!("{:.2} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    } else if bytes >= 1024 * 1024 {
        format!("{:.2} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.2} KB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

// ============================================================================
// Latency Percentile Measurement
// ============================================================================

/// Measures latency percentiles (p50, p95, p99) for search operations.
fn measure_percentiles(index: &HnswIndex, num_queries: usize, k: usize) -> (f64, f64, f64) {
    let mut latencies: Vec<Duration> = Vec::with_capacity(num_queries);

    for i in 0..num_queries {
        let query = generate_normalized_vector(index.dimension(), 1_000_000 + i as u64);
        let start = Instant::now();
        let _ = black_box(index.search(&query, k));
        latencies.push(start.elapsed());
    }

    latencies.sort();

    let p50_idx = (num_queries as f64 * 0.50) as usize;
    let p95_idx = (num_queries as f64 * 0.95) as usize;
    let p99_idx = (num_queries as f64 * 0.99) as usize;

    let p50 = latencies[p50_idx.min(num_queries - 1)].as_secs_f64() * 1000.0; // ms
    let p95 = latencies[p95_idx.min(num_queries - 1)].as_secs_f64() * 1000.0;
    let p99 = latencies[p99_idx.min(num_queries - 1)].as_secs_f64() * 1000.0;

    (p50, p95, p99)
}

// ============================================================================
// Scalability Benchmark: Insert Performance at Scale
// ============================================================================

fn bench_scalability_insert(c: &mut Criterion) {
    let mut group = c.benchmark_group("scalability_insert");
    group.sampling_mode(SamplingMode::Flat);
    group.sample_size(10); // Fewer samples for long-running benchmarks

    for &size in DATASET_SIZES {
        group.throughput(Throughput::Elements(size as u64));

        group.bench_with_input(
            BenchmarkId::new("vectors", format!("{}x{}d", size / 1000, DIMENSION)),
            &size,
            |b, &size| {
                b.iter_custom(|iters| {
                    let mut total = Duration::ZERO;
                    for _ in 0..iters {
                        let mem_before = get_memory_usage();
                        let index = HnswIndex::new(DIMENSION, DistanceMetric::Cosine);

                        let start = Instant::now();
                        for i in 0..size {
                            let vector = generate_normalized_vector(DIMENSION, i as u64);
                            index.insert(i as u64, &vector);
                        }
                        total += start.elapsed();

                        let mem_after = get_memory_usage();
                        let mem_delta = mem_after.saturating_sub(mem_before);

                        // Print memory info on first iteration
                        if iters == 1 {
                            eprintln!(
                                "\n[{}k vectors] Memory: {} (delta: {})",
                                size / 1000,
                                format_bytes(mem_after),
                                format_bytes(mem_delta)
                            );
                        }

                        black_box(index.len());
                    }
                    total
                });
            },
        );
    }

    group.finish();
}

// ============================================================================
// Scalability Benchmark: Search Latency with Percentiles
// ============================================================================

fn bench_scalability_search_percentiles(c: &mut Criterion) {
    let mut group = c.benchmark_group("scalability_search_percentiles");
    group.sampling_mode(SamplingMode::Flat);
    group.sample_size(10);

    for &size in DATASET_SIZES {
        // Pre-build index
        eprintln!("\nBuilding index with {}k vectors...", size / 1000);
        let index = HnswIndex::new(DIMENSION, DistanceMetric::Cosine);
        for i in 0..size {
            let vector = generate_normalized_vector(DIMENSION, i as u64);
            index.insert(i as u64, &vector);
        }
        eprintln!("Index built. Running latency tests...");

        // Measure percentiles
        let num_queries = 1000;
        let (p50, p95, p99) = measure_percentiles(&index, num_queries, 10);
        eprintln!(
            "[{}k vectors] Latency: p50={:.3}ms, p95={:.3}ms, p99={:.3}ms",
            size / 1000,
            p50,
            p95,
            p99
        );

        group.bench_with_input(
            BenchmarkId::new("search_top10", format!("{}k", size / 1000)),
            &size,
            |b, _| {
                b.iter(|| {
                    let query = generate_normalized_vector(DIMENSION, 999_999);
                    let results = index.search(&query, 10);
                    black_box(results)
                });
            },
        );
    }

    group.finish();
}

// ============================================================================
// Concurrent Query Benchmark
// ============================================================================

fn bench_concurrent_queries(c: &mut Criterion) {
    let mut group = c.benchmark_group("concurrent_queries");
    group.sampling_mode(SamplingMode::Flat);
    group.sample_size(10);

    // Use 100k for concurrent tests (good balance of size vs build time)
    let dataset_size = 100_000;

    eprintln!(
        "\nBuilding index with {}k vectors for concurrent tests...",
        dataset_size / 1000
    );
    let index = Arc::new(HnswIndex::new(DIMENSION, DistanceMetric::Cosine));
    for i in 0..dataset_size {
        let vector = generate_normalized_vector(DIMENSION, i as u64);
        index.insert(i as u64, &vector);
    }
    eprintln!("Index built.");

    for &num_threads in THREAD_COUNTS {
        let total_queries = num_threads * QUERIES_PER_THREAD;
        group.throughput(Throughput::Elements(total_queries as u64));

        group.bench_with_input(
            BenchmarkId::new("threads", num_threads),
            &num_threads,
            |b, &num_threads| {
                b.iter_custom(|iters| {
                    let mut total = Duration::ZERO;

                    for _ in 0..iters {
                        let start = Instant::now();
                        let handles: Vec<_> = (0..num_threads)
                            .map(|t| {
                                let index = Arc::clone(&index);
                                thread::spawn(move || {
                                    for q in 0..QUERIES_PER_THREAD {
                                        let seed = (t * QUERIES_PER_THREAD + q) as u64 + 1_000_000;
                                        let query = generate_normalized_vector(DIMENSION, seed);
                                        let results = index.search(&query, 10);
                                        black_box(results);
                                    }
                                })
                            })
                            .collect();

                        for handle in handles {
                            handle.join().expect("Thread panicked");
                        }
                        total += start.elapsed();
                    }

                    total
                });
            },
        );
    }

    group.finish();
}

// ============================================================================
// Memory Usage Benchmark
// ============================================================================

fn bench_memory_usage(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory_usage");
    group.sampling_mode(SamplingMode::Flat);
    group.sample_size(10);

    for &size in &[10_000usize, 50_000, 100_000] {
        group.bench_with_input(
            BenchmarkId::new("index_build", format!("{}k", size / 1000)),
            &size,
            |b, &size| {
                b.iter_custom(|iters| {
                    let mut total = Duration::ZERO;

                    for iter in 0..iters {
                        let mem_before = get_memory_usage();

                        let start = Instant::now();
                        let index = HnswIndex::new(DIMENSION, DistanceMetric::Cosine);
                        for i in 0..size {
                            let vector = generate_normalized_vector(DIMENSION, i as u64);
                            index.insert(i as u64, &vector);
                        }
                        total += start.elapsed();

                        let mem_after = get_memory_usage();

                        // Report memory on first iteration
                        if iter == 0 {
                            let mem_per_vector = if size > 0 {
                                (mem_after.saturating_sub(mem_before)) / size
                            } else {
                                0
                            };
                            eprintln!(
                                "\n[{}k vectors] Total: {}, Per-vector: {} bytes",
                                size / 1000,
                                format_bytes(mem_after.saturating_sub(mem_before)),
                                mem_per_vector
                            );
                        }

                        black_box(index.len());
                        // Drop index to free memory before next iteration
                        drop(index);
                    }

                    total
                });
            },
        );
    }

    group.finish();
}

// ============================================================================
// Criterion Configuration
// ============================================================================

criterion_group! {
    name = scalability_benches;
    config = Criterion::default()
        .measurement_time(Duration::from_secs(30))
        .warm_up_time(Duration::from_secs(5));
    targets =
        bench_scalability_insert,
        bench_scalability_search_percentiles,
        bench_concurrent_queries,
        bench_memory_usage
}

criterion_main!(scalability_benches);
