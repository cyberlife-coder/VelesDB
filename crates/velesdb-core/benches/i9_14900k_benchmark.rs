//! Benchmark spécifique pour CPU haut de gamme (i9-14900K, AVX-512 natif)
//!
//! Ce benchmark compare les performances AVX-512 natif vs AVX2
//! et mesure l'impact du 4-accumulateur sur ce hardware.

#![allow(clippy::cast_precision_loss)]

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};

fn benchmark_dot_product_i9_14900k(c: &mut Criterion) {
    let mut group = c.benchmark_group("i9-14900k_dot_product");

    // Dimensions typiques pour embeddings modernes
    for size in [384usize, 512, 768, 1024, 1536, 2048, 3072, 4096] {
        let a: Vec<f32> = (0..size).map(|i| (i as f32) * 0.001).collect();
        let b: Vec<f32> = (0..size).map(|i| ((size - 1 - i) as f32) * 0.001).collect();

        group.throughput(Throughput::Elements(size as u64));

        group.bench_with_input(
            BenchmarkId::new("avx512_native", size),
            &(a, b),
            |bench, (a, b)| {
                bench.iter(|| {
                    // Appel via dot_product_native qui utilisera AVX-512 sur i9-14900K
                    black_box(velesdb_core::simd_native_native::dot_product_native(a, b))
                });
            },
        );
    }

    group.finish();
}

fn benchmark_squared_l2_i9_14900k(c: &mut Criterion) {
    let mut group = c.benchmark_group("i9-14900k_squared_l2");

    for size in [384usize, 768, 1536, 3072] {
        let a: Vec<f32> = (0..size).map(|i| (i as f32) * 0.001).collect();
        let b: Vec<f32> = (0..size).map(|i| ((size - 1 - i) as f32) * 0.001).collect();

        group.throughput(Throughput::Elements(size as u64));

        group.bench_with_input(
            BenchmarkId::new("avx512_native", size),
            &(a, b),
            |bench, (a, b)| {
                bench.iter(|| black_box(velesdb_core::simd_native_native::squared_l2_native(a, b)));
            },
        );
    }

    group.finish();
}

fn benchmark_batch_dot_product(c: &mut Criterion) {
    let mut group = c.benchmark_group("i9-14900k_batch_dot");

    // Batch de queries pour mesurer throughput
    let query: Vec<f32> = (0..768).map(|i| (i as f32) * 0.001).collect();
    let vectors: Vec<Vec<f32>> = (0..100)
        .map(|j| (0..768).map(|i| ((i + j) as f32) * 0.001).collect())
        .collect();

    group.bench_function("batch_100x768", |bench| {
        bench.iter(|| {
            let results: Vec<f32> = vectors
                .iter()
                .map(|v| velesdb_core::simd_native_native::dot_product_native(&query, v))
                .collect();
            black_box(results)
        });
    });

    group.finish();
}

fn benchmark_throughput_millions(c: &mut Criterion) {
    let mut group = c.benchmark_group("i9-14900k_throughput");

    // Mesurer le throughput en millions d'opérations par seconde
    let sizes = vec![(128, "128D"), (384, "384D"), (768, "768D"), (1536, "1536D")];

    for (size, label) in sizes {
        let a: Vec<f32> = vec![1.0; size];
        let b: Vec<f32> = vec![1.0; size];

        group.throughput(Throughput::Elements(size as u64));

        group.bench_with_input(BenchmarkId::new("mops", label), &(a, b), |bench, (a, b)| {
            bench.iter(|| black_box(velesdb_core::simd_native_native::dot_product_native(a, b)));
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    benchmark_dot_product_i9_14900k,
    benchmark_squared_l2_i9_14900k,
    benchmark_batch_dot_product,
    benchmark_throughput_millions
);
criterion_main!(benches);
