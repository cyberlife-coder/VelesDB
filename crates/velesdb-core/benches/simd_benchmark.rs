//! Benchmark comparing auto-vectorized vs explicit SIMD implementations.
//!
//! Run with: `cargo bench --bench simd_benchmark`

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use velesdb_core::simd::{cosine_similarity_fast, dot_product_fast, euclidean_distance_fast};
use velesdb_core::simd_explicit::{
    cosine_similarity_simd, dot_product_simd, euclidean_distance_simd,
};

fn generate_vector(dim: usize, seed: f32) -> Vec<f32> {
    #[allow(clippy::cast_precision_loss)]
    (0..dim).map(|i| (seed + i as f32 * 0.1).sin()).collect()
}

fn bench_dot_product(c: &mut Criterion) {
    let mut group = c.benchmark_group("dot_product");

    for dim in [128, 384, 768, 1536].iter() {
        let a = generate_vector(*dim, 0.0);
        let b = generate_vector(*dim, 1.0);

        group.bench_with_input(BenchmarkId::new("auto_vec", dim), dim, |bencher, _| {
            bencher.iter(|| dot_product_fast(black_box(&a), black_box(&b)));
        });

        group.bench_with_input(BenchmarkId::new("explicit_simd", dim), dim, |bencher, _| {
            bencher.iter(|| dot_product_simd(black_box(&a), black_box(&b)));
        });
    }

    group.finish();
}

fn bench_euclidean_distance(c: &mut Criterion) {
    let mut group = c.benchmark_group("euclidean_distance");

    for dim in [128, 384, 768, 1536].iter() {
        let a = generate_vector(*dim, 0.0);
        let b = generate_vector(*dim, 1.0);

        group.bench_with_input(BenchmarkId::new("auto_vec", dim), dim, |bencher, _| {
            bencher.iter(|| euclidean_distance_fast(black_box(&a), black_box(&b)));
        });

        group.bench_with_input(BenchmarkId::new("explicit_simd", dim), dim, |bencher, _| {
            bencher.iter(|| euclidean_distance_simd(black_box(&a), black_box(&b)));
        });
    }

    group.finish();
}

fn bench_cosine_similarity(c: &mut Criterion) {
    let mut group = c.benchmark_group("cosine_similarity");

    for dim in [128, 384, 768, 1536].iter() {
        let a = generate_vector(*dim, 0.0);
        let b = generate_vector(*dim, 1.0);

        group.bench_with_input(BenchmarkId::new("auto_vec", dim), dim, |bencher, _| {
            bencher.iter(|| cosine_similarity_fast(black_box(&a), black_box(&b)));
        });

        group.bench_with_input(BenchmarkId::new("explicit_simd", dim), dim, |bencher, _| {
            bencher.iter(|| cosine_similarity_simd(black_box(&a), black_box(&b)));
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_dot_product,
    bench_euclidean_distance,
    bench_cosine_similarity
);
criterion_main!(benches);
