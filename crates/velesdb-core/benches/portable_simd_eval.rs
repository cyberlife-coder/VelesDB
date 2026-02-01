//! Benchmark: `simd_ops` vs scalar comparison (EPIC-078/US-005)
//!
//! Run with:
//! ```bash
//! cargo bench --bench portable_simd_eval
//! ```
#![allow(clippy::cast_precision_loss)]

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use velesdb_core::{simd_ops, DistanceMetric};

#[allow(clippy::cast_precision_loss)]
fn random_vec(len: usize) -> Vec<f32> {
    (0..len).map(|i| (i as f32 * 0.001) % 1.0).collect()
}

fn bench_l2_distance(c: &mut Criterion) {
    let mut group = c.benchmark_group("l2_distance_comparison");

    for dim in &[128, 384, 768, 1536] {
        group.throughput(Throughput::Elements(*dim as u64));

        let a = random_vec(*dim);
        let b = random_vec(*dim);

        group.bench_with_input(BenchmarkId::new("simd_ops", dim), dim, |bencher, _| {
            bencher.iter(|| {
                simd_ops::similarity(DistanceMetric::Euclidean, black_box(&a), black_box(&b))
            });
        });

        // Scalar baseline for comparison
        group.bench_with_input(BenchmarkId::new("scalar", dim), dim, |bencher, _| {
            bencher.iter(|| {
                let sum: f32 = a.iter().zip(b.iter()).map(|(x, y)| (x - y).powi(2)).sum();
                black_box(sum.sqrt())
            });
        });
    }

    group.finish();
}

fn bench_dot_product(c: &mut Criterion) {
    let mut group = c.benchmark_group("dot_product_comparison");

    for dim in &[128, 384, 768, 1536] {
        group.throughput(Throughput::Elements(*dim as u64));

        let a = random_vec(*dim);
        let b = random_vec(*dim);

        group.bench_with_input(BenchmarkId::new("simd_ops", dim), dim, |bencher, _| {
            bencher.iter(|| simd_ops::dot_product(black_box(&a), black_box(&b)));
        });

        group.bench_with_input(BenchmarkId::new("scalar", dim), dim, |bencher, _| {
            bencher.iter(|| {
                let sum: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
                black_box(sum)
            });
        });
    }

    group.finish();
}

fn bench_cosine_similarity(c: &mut Criterion) {
    let mut group = c.benchmark_group("cosine_similarity_comparison");

    for dim in &[128, 384, 768, 1536] {
        group.throughput(Throughput::Elements(*dim as u64));

        let a = random_vec(*dim);
        let b = random_vec(*dim);

        group.bench_with_input(BenchmarkId::new("simd_ops", dim), dim, |bencher, _| {
            bencher.iter(|| {
                simd_ops::similarity(DistanceMetric::Cosine, black_box(&a), black_box(&b))
            });
        });

        group.bench_with_input(BenchmarkId::new("scalar", dim), dim, |bencher, _| {
            bencher.iter(|| {
                let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
                let a_norm: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
                let b_norm: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
                black_box(dot / (a_norm * b_norm))
            });
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_l2_distance,
    bench_dot_product,
    bench_cosine_similarity
);
criterion_main!(benches);
