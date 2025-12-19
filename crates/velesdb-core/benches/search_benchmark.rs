//! Benchmark suite for VelesDB-Core search operations.
//!
//! Run with: `cargo bench --bench search_benchmark`
//!
//! Compares baseline (naive) vs optimized (SIMD/fused) implementations.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use velesdb_core::simd;

fn generate_random_vector(dim: usize) -> Vec<f32> {
    #[allow(clippy::cast_precision_loss)]
    (0..dim).map(|i| (i as f32 * 0.1).sin()).collect()
}

/// Benchmark cosine distance: baseline (3-pass) vs optimized (single-pass fused)
fn bench_cosine_distance(c: &mut Criterion) {
    let mut group = c.benchmark_group("cosine_distance");
    let dim = 768;
    let vec_a = generate_random_vector(dim);
    let vec_b = generate_random_vector(dim);

    group.bench_function(BenchmarkId::new("baseline", "768d"), |b| {
        b.iter(|| {
            let dot: f32 = vec_a.iter().zip(&vec_b).map(|(a, b)| a * b).sum();
            let norm_a: f32 = vec_a.iter().map(|x| x * x).sum::<f32>().sqrt();
            let norm_b: f32 = vec_b.iter().map(|x| x * x).sum::<f32>().sqrt();
            black_box(dot / (norm_a * norm_b))
        });
    });

    group.bench_function(BenchmarkId::new("optimized", "768d"), |b| {
        b.iter(|| black_box(simd::cosine_similarity_fast(&vec_a, &vec_b)));
    });

    group.finish();
}

/// Benchmark euclidean distance: baseline vs optimized (unrolled)
fn bench_euclidean_distance(c: &mut Criterion) {
    let mut group = c.benchmark_group("euclidean_distance");
    let dim = 768;
    let vec_a = generate_random_vector(dim);
    let vec_b = generate_random_vector(dim);

    group.bench_function(BenchmarkId::new("baseline", "768d"), |b| {
        b.iter(|| {
            let sum: f32 = vec_a.iter().zip(&vec_b).map(|(a, b)| (a - b).powi(2)).sum();
            black_box(sum.sqrt())
        });
    });

    group.bench_function(BenchmarkId::new("optimized", "768d"), |b| {
        b.iter(|| black_box(simd::euclidean_distance_fast(&vec_a, &vec_b)));
    });

    group.finish();
}

/// Benchmark normalization: baseline (allocating) vs optimized (in-place)
fn bench_normalization(c: &mut Criterion) {
    let mut group = c.benchmark_group("normalize");
    let dim = 768;

    group.bench_function(BenchmarkId::new("baseline_alloc", "768d"), |b| {
        let vec = generate_random_vector(dim);
        b.iter(|| {
            let norm: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
            let normalized: Vec<f32> = vec.iter().map(|x| x / norm).collect();
            black_box(normalized)
        });
    });

    group.bench_function(BenchmarkId::new("optimized_inplace", "768d"), |b| {
        b.iter_batched(
            || generate_random_vector(dim),
            |mut vec| {
                simd::normalize_inplace(&mut vec);
                black_box(vec)
            },
            criterion::BatchSize::SmallInput,
        );
    });

    group.finish();
}

/// Benchmark dot product: baseline vs optimized (unrolled)
fn bench_dot_product(c: &mut Criterion) {
    let mut group = c.benchmark_group("dot_product");
    let dim = 768;
    let vec_a = generate_random_vector(dim);
    let vec_b = generate_random_vector(dim);

    group.bench_function(BenchmarkId::new("baseline", "768d"), |b| {
        b.iter(|| {
            let dot: f32 = vec_a.iter().zip(&vec_b).map(|(a, b)| a * b).sum();
            black_box(dot)
        });
    });

    group.bench_function(BenchmarkId::new("optimized", "768d"), |b| {
        b.iter(|| black_box(simd::dot_product_fast(&vec_a, &vec_b)));
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_cosine_distance,
    bench_euclidean_distance,
    bench_normalization,
    bench_dot_product
);
criterion_main!(benches);
