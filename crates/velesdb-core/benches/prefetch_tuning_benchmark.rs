//! EPIC-033/US-005: CPU Prefetch Distance Tuning Benchmark
//!
//! This benchmark harness determines optimal prefetch distances for different:
//! - Vector dimensions (128D, 384D, 768D, 1536D, 3072D)
//! - CPU architectures (via runtime detection)
//! - Batch sizes (common HNSW `ef_search` values)
//!
//! # Research Background (arXiv references)
//!
//! - arXiv:2505.07621 "Bang for the Buck": CPU cache critical for memory-bound vector search
//! - arXiv:2508.03016 "KBest": Hardware-aware prefetch for vector search
//! - Intel Optimization Guide: Prefetch distance depends on cache hierarchy
//!
//! # Run with
//!
//! ```bash
//! cargo bench --bench prefetch_tuning_benchmark
//! cargo bench --bench prefetch_tuning_benchmark -- --save-baseline cpu_baseline
//! ```

#![allow(clippy::cast_precision_loss)]

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};

// Import VelesDB SIMD functions
use velesdb_core::simd_native::{
    calculate_prefetch_distance, cosine_similarity_native, dot_product_native, prefetch_vector,
};

/// L2 cache line size in bytes
const L2_CACHE_LINE: usize = 64;

/// Dimensions to test (covers common embedding models)
const DIMENSIONS: &[usize] = &[128, 384, 768, 1536, 3072];

/// Prefetch distances to evaluate
const PREFETCH_DISTANCES: &[usize] = &[0, 2, 4, 8, 16, 32];

/// Batch sizes (simulates HNSW `ef_search` values)
const BATCH_SIZES: &[usize] = &[16, 32, 64, 128, 256];

/// Generates deterministic test vectors
fn generate_vectors(dim: usize, count: usize) -> Vec<Vec<f32>> {
    (0..count)
        .map(|seed| {
            (0..dim)
                .map(|i| ((seed as f32 * 0.1 + i as f32 * 0.01).sin() + 1.0) / 2.0)
                .collect()
        })
        .collect()
}

/// Generates a query vector
fn generate_query(dim: usize) -> Vec<f32> {
    (0..dim)
        .map(|i| ((42.0_f32 * 0.1 + i as f32 * 0.01).cos() + 1.0) / 2.0)
        .collect()
}

/// Batch dot product with configurable prefetch distance
#[inline(never)]
fn batch_dot_with_prefetch(
    candidates: &[&[f32]],
    query: &[f32],
    prefetch_distance: usize,
) -> Vec<f32> {
    let mut results = Vec::with_capacity(candidates.len());

    for (i, candidate) in candidates.iter().enumerate() {
        // Prefetch ahead if distance > 0
        if prefetch_distance > 0 && i + prefetch_distance < candidates.len() {
            prefetch_vector(candidates[i + prefetch_distance]);
        }
        results.push(dot_product_native(candidate, query));
    }

    results
}

/// Batch cosine similarity with configurable prefetch distance
#[inline(never)]
fn batch_cosine_with_prefetch(
    candidates: &[&[f32]],
    query: &[f32],
    prefetch_distance: usize,
) -> Vec<f32> {
    let mut results = Vec::with_capacity(candidates.len());

    for (i, candidate) in candidates.iter().enumerate() {
        if prefetch_distance > 0 && i + prefetch_distance < candidates.len() {
            prefetch_vector(candidates[i + prefetch_distance]);
        }
        results.push(cosine_similarity_native(candidate, query));
    }

    results
}

/// Benchmark: Prefetch distance sweep for dot product
fn bench_prefetch_dot_product(c: &mut Criterion) {
    let mut group = c.benchmark_group("prefetch_dot_product");
    group.sample_size(100);

    for &dim in DIMENSIONS {
        let vectors = generate_vectors(dim, 256);
        let query = generate_query(dim);
        let candidates: Vec<&[f32]> = vectors.iter().map(Vec::as_slice).collect();

        // Report throughput in elements (vectors processed)
        group.throughput(Throughput::Elements(candidates.len() as u64));

        for &prefetch_dist in PREFETCH_DISTANCES {
            let id = format!("dim={dim}/prefetch={prefetch_dist}");
            group.bench_with_input(BenchmarkId::new("sweep", &id), &prefetch_dist, |b, &pd| {
                b.iter(|| {
                    black_box(batch_dot_with_prefetch(
                        black_box(&candidates),
                        black_box(&query),
                        pd,
                    ))
                });
            });
        }

        // Also bench the calculated optimal distance
        let optimal = calculate_prefetch_distance(dim);
        let id = format!("dim={dim}/prefetch=optimal({optimal})");
        group.bench_with_input(BenchmarkId::new("optimal", &id), &optimal, |b, &pd| {
            b.iter(|| {
                black_box(batch_dot_with_prefetch(
                    black_box(&candidates),
                    black_box(&query),
                    pd,
                ))
            });
        });
    }

    group.finish();
}

/// Benchmark: Prefetch distance sweep for cosine similarity
fn bench_prefetch_cosine(c: &mut Criterion) {
    let mut group = c.benchmark_group("prefetch_cosine");
    group.sample_size(100);

    for &dim in DIMENSIONS {
        let vectors = generate_vectors(dim, 256);
        let query = generate_query(dim);
        let candidates: Vec<&[f32]> = vectors.iter().map(Vec::as_slice).collect();

        group.throughput(Throughput::Elements(candidates.len() as u64));

        for &prefetch_dist in PREFETCH_DISTANCES {
            let id = format!("dim={dim}/prefetch={prefetch_dist}");
            group.bench_with_input(BenchmarkId::new("sweep", &id), &prefetch_dist, |b, &pd| {
                b.iter(|| {
                    black_box(batch_cosine_with_prefetch(
                        black_box(&candidates),
                        black_box(&query),
                        pd,
                    ))
                });
            });
        }
    }

    group.finish();
}

/// Benchmark: Impact of batch size on prefetch effectiveness
fn bench_batch_size_impact(c: &mut Criterion) {
    let mut group = c.benchmark_group("batch_size_impact");
    group.sample_size(50);

    let dim = 768; // Common embedding dimension
    let vectors = generate_vectors(dim, 512);
    let query = generate_query(dim);

    for &batch_size in BATCH_SIZES {
        let candidates: Vec<&[f32]> = vectors.iter().take(batch_size).map(Vec::as_slice).collect();
        group.throughput(Throughput::Elements(batch_size as u64));

        // No prefetch baseline
        let id = format!("batch={batch_size}/prefetch=0");
        group.bench_with_input(BenchmarkId::new("no_prefetch", &id), &batch_size, |b, _| {
            b.iter(|| {
                black_box(batch_dot_with_prefetch(
                    black_box(&candidates),
                    black_box(&query),
                    0,
                ))
            });
        });

        // Optimal prefetch
        let optimal = calculate_prefetch_distance(dim);
        let id = format!("batch={batch_size}/prefetch={optimal}");
        group.bench_with_input(
            BenchmarkId::new("with_prefetch", &id),
            &batch_size,
            |b, _| {
                b.iter(|| {
                    black_box(batch_dot_with_prefetch(
                        black_box(&candidates),
                        black_box(&query),
                        optimal,
                    ))
                });
            },
        );
    }

    group.finish();
}

/// Benchmark: High-dimension focus (384D-3072D regression analysis)
fn bench_high_dimension_prefetch(c: &mut Criterion) {
    let mut group = c.benchmark_group("high_dimension_prefetch");
    group.sample_size(50);

    // Focus on high dimensions where regression was observed
    let high_dims: &[usize] = &[384, 768, 1536, 3072];

    for &dim in high_dims {
        let vectors = generate_vectors(dim, 128);
        let query = generate_query(dim);
        let candidates: Vec<&[f32]> = vectors.iter().map(Vec::as_slice).collect();

        group.throughput(Throughput::Elements(candidates.len() as u64));

        // Calculate vector size for analysis
        let vector_bytes = dim * std::mem::size_of::<f32>();
        let cache_lines = vector_bytes / L2_CACHE_LINE;

        // Test aggressive prefetch for large vectors
        let aggressive_prefetch = (cache_lines / 2).clamp(4, 32);

        let id = format!("dim={dim}/bytes={vector_bytes}/lines={cache_lines}");

        // Baseline (no prefetch)
        group.bench_with_input(BenchmarkId::new("no_prefetch", &id), &dim, |b, _| {
            b.iter(|| {
                black_box(batch_cosine_with_prefetch(
                    black_box(&candidates),
                    black_box(&query),
                    0,
                ))
            });
        });

        // Standard formula
        let standard = calculate_prefetch_distance(dim);
        group.bench_with_input(
            BenchmarkId::new(format!("standard({standard})"), &id),
            &dim,
            |b, _| {
                b.iter(|| {
                    black_box(batch_cosine_with_prefetch(
                        black_box(&candidates),
                        black_box(&query),
                        standard,
                    ))
                });
            },
        );

        // Aggressive for high-dim
        group.bench_with_input(
            BenchmarkId::new(format!("aggressive({aggressive_prefetch})"), &id),
            &dim,
            |b, _| {
                b.iter(|| {
                    black_box(batch_cosine_with_prefetch(
                        black_box(&candidates),
                        black_box(&query),
                        aggressive_prefetch,
                    ))
                });
            },
        );
    }

    group.finish();
}

/// Benchmark: Cache miss analysis (measures memory-bound behavior)
fn bench_cache_miss_analysis(c: &mut Criterion) {
    let mut group = c.benchmark_group("cache_miss_analysis");
    group.sample_size(30);

    // Large dataset to ensure cache misses
    let dim = 768;
    let count = 2048; // ~6MB of vectors, exceeds L3 cache
    let vectors = generate_vectors(dim, count);
    let query = generate_query(dim);

    // Random access pattern (worst case for cache)
    let random_indices: Vec<usize> = {
        let mut indices: Vec<usize> = (0..count).collect();
        // Simple shuffle using LCG
        for i in (1..count).rev() {
            let j = (i * 1_103_515_245 + 12345) % (i + 1);
            indices.swap(i, j);
        }
        indices
    };

    // Sequential access (best case)
    let sequential_indices: Vec<usize> = (0..count).collect();

    group.throughput(Throughput::Elements(count as u64));

    // Sequential with prefetch
    let optimal = calculate_prefetch_distance(dim);
    group.bench_function(
        BenchmarkId::new("sequential", format!("prefetch={optimal}")),
        |b| {
            let candidates: Vec<&[f32]> = sequential_indices
                .iter()
                .map(|&i| vectors[i].as_slice())
                .collect();
            b.iter(|| {
                black_box(batch_dot_with_prefetch(
                    black_box(&candidates),
                    black_box(&query),
                    optimal,
                ))
            });
        },
    );

    // Random with prefetch (stress test)
    group.bench_function(
        BenchmarkId::new("random", format!("prefetch={optimal}")),
        |b| {
            let candidates: Vec<&[f32]> = random_indices
                .iter()
                .map(|&i| vectors[i].as_slice())
                .collect();
            b.iter(|| {
                black_box(batch_dot_with_prefetch(
                    black_box(&candidates),
                    black_box(&query),
                    optimal,
                ))
            });
        },
    );

    // Random without prefetch (baseline)
    group.bench_function(BenchmarkId::new("random", "no_prefetch"), |b| {
        let candidates: Vec<&[f32]> = random_indices
            .iter()
            .map(|&i| vectors[i].as_slice())
            .collect();
        b.iter(|| {
            black_box(batch_dot_with_prefetch(
                black_box(&candidates),
                black_box(&query),
                0,
            ))
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_prefetch_dot_product,
    bench_prefetch_cosine,
    bench_batch_size_impact,
    bench_high_dimension_prefetch,
    bench_cache_miss_analysis,
);

criterion_main!(benches);
