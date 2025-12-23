//! Performance benchmarks for optimizations.
//!
//! Measures:
//! - `ContiguousVectors` vs `Vec<Vec<f32>>` access patterns
//! - Prefetch impact on HNSW-like traversal
//! - Batch distance computations

#![allow(
    clippy::cast_precision_loss,
    clippy::semicolon_if_nothing_returned,
    clippy::similar_names,
    clippy::doc_markdown
)]

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use velesdb_core::perf_optimizations::ContiguousVectors;
use velesdb_core::simd_avx512::dot_product_auto;

/// Generate random vectors for benchmarking.
fn generate_vectors(count: usize, dimension: usize) -> Vec<Vec<f32>> {
    (0..count)
        .map(|i| {
            (0..dimension)
                .map(|j| ((i * dimension + j) % 1000) as f32 / 1000.0)
                .collect()
        })
        .collect()
}

/// Benchmark: ContiguousVectors vs Vec<Vec<f32>> random access
fn bench_random_access(c: &mut Criterion) {
    let mut group = c.benchmark_group("random_access");
    let dimension = 768;
    let count = 10_000;

    let vectors = generate_vectors(count, dimension);
    let mut contiguous = ContiguousVectors::new(dimension, count);
    for v in &vectors {
        contiguous.push(v);
    }

    // Random access pattern (simulates HNSW traversal)
    let indices: Vec<usize> = (0..1000).map(|i| (i * 7) % count).collect();

    group.throughput(Throughput::Elements(indices.len() as u64));

    group.bench_function("vec_vec_random", |b| {
        b.iter(|| {
            let mut sum = 0.0f32;
            for &idx in &indices {
                sum += vectors[idx][0];
            }
            black_box(sum)
        })
    });

    group.bench_function("contiguous_random", |b| {
        b.iter(|| {
            let mut sum = 0.0f32;
            for &idx in &indices {
                if let Some(v) = contiguous.get(idx) {
                    sum += v[0];
                }
            }
            black_box(sum)
        })
    });

    group.bench_function("contiguous_prefetch", |b| {
        b.iter(|| {
            let mut sum = 0.0f32;
            for (i, &idx) in indices.iter().enumerate() {
                // Prefetch ahead
                if i + 4 < indices.len() {
                    contiguous.prefetch(indices[i + 4]);
                }
                if let Some(v) = contiguous.get(idx) {
                    sum += v[0];
                }
            }
            black_box(sum)
        })
    });

    group.finish();
}

/// Benchmark: Batch dot products
fn bench_batch_dot_products(c: &mut Criterion) {
    let mut group = c.benchmark_group("batch_dot_products");

    for dimension in [128, 768, 1536] {
        let count = 100;
        let vectors = generate_vectors(count, dimension);
        let query: Vec<f32> = (0..dimension)
            .map(|i| i as f32 / dimension as f32)
            .collect();

        let mut contiguous = ContiguousVectors::new(dimension, count);
        for v in &vectors {
            contiguous.push(v);
        }

        let indices: Vec<usize> = (0..count).collect();

        group.throughput(Throughput::Elements(count as u64));

        // Individual dot products (baseline)
        group.bench_with_input(
            BenchmarkId::new("individual", dimension),
            &dimension,
            |b, _| {
                b.iter(|| {
                    let results: Vec<f32> = vectors
                        .iter()
                        .map(|v| dot_product_auto(v, &query))
                        .collect();
                    black_box(results)
                })
            },
        );

        // Batch with prefetch (optimized)
        group.bench_with_input(
            BenchmarkId::new("batch_prefetch", dimension),
            &dimension,
            |b, _| {
                b.iter(|| {
                    let results = contiguous.batch_dot_products(&indices, &query);
                    black_box(results)
                })
            },
        );
    }

    group.finish();
}

/// Benchmark: Contiguous storage insert throughput
fn bench_insert_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("insert_throughput");

    for dimension in [128, 768, 1536] {
        let count = 1000;
        let vectors = generate_vectors(count, dimension);

        group.throughput(Throughput::Elements(count as u64));

        group.bench_with_input(
            BenchmarkId::new("contiguous_push", dimension),
            &dimension,
            |b, &dim| {
                b.iter(|| {
                    let mut cv = ContiguousVectors::new(dim, count);
                    for v in &vectors {
                        cv.push(v);
                    }
                    black_box(cv.len())
                })
            },
        );

        group.bench_with_input(
            BenchmarkId::new("contiguous_push_batch", dimension),
            &dimension,
            |b, &dim| {
                b.iter(|| {
                    let mut cv = ContiguousVectors::new(dim, count);
                    let refs: Vec<&[f32]> = vectors.iter().map(Vec::as_slice).collect();
                    let added = cv.push_batch(refs.into_iter());
                    black_box(added)
                })
            },
        );
    }

    group.finish();
}

/// Benchmark: Memory efficiency comparison
fn bench_memory_efficiency(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory_comparison");
    let dimension = 768;
    let count = 10_000;

    let vectors = generate_vectors(count, dimension);

    // Measure Vec<Vec<f32>> overhead
    group.bench_function("vec_vec_allocation", |b| {
        b.iter(|| {
            let v: Vec<Vec<f32>> = vectors.clone();
            black_box(v.len())
        })
    });

    // Measure ContiguousVectors allocation
    group.bench_function("contiguous_allocation", |b| {
        b.iter(|| {
            let mut cv = ContiguousVectors::new(dimension, count);
            for v in &vectors {
                cv.push(v);
            }
            black_box(cv.len())
        })
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_random_access,
    bench_batch_dot_products,
    bench_insert_throughput,
    bench_memory_efficiency,
);
criterion_main!(benches);
