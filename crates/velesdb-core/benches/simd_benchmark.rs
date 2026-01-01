//! Benchmark comparing auto-vectorized vs explicit SIMD implementations.
//!
//! Run with: `cargo bench --bench simd_benchmark`

#![allow(clippy::similar_names)]
#![allow(clippy::cast_precision_loss)]

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use velesdb_core::simd::{
    cosine_similarity_fast, dot_product_fast, euclidean_distance_fast, hamming_distance_fast,
    jaccard_similarity_fast,
};
use velesdb_core::simd_explicit::{
    cosine_similarity_simd, dot_product_simd, euclidean_distance_simd, hamming_distance_binary,
    hamming_distance_binary_fast, hamming_distance_simd,
};

fn generate_vector(dim: usize, seed: f32) -> Vec<f32> {
    #[allow(clippy::cast_precision_loss)]
    (0..dim).map(|i| (seed + i as f32 * 0.1).sin()).collect()
}

fn bench_dot_product(c: &mut Criterion) {
    let mut group = c.benchmark_group("dot_product");

    for dim in &[128, 384, 768, 1536, 3072] {
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

    for dim in &[128, 384, 768, 1536, 3072] {
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

    for dim in &[128, 384, 768, 1536, 3072] {
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

fn generate_binary_vector(dim: usize, seed: usize) -> Vec<f32> {
    (0..dim)
        .map(|i| if (i + seed) % 3 == 0 { 1.0 } else { 0.0 })
        .collect()
}

fn generate_packed_binary(num_u64: usize, seed: u64) -> Vec<u64> {
    (0..num_u64)
        .map(|i| (i as u64).wrapping_mul(seed))
        .collect()
}

fn bench_hamming_f32(c: &mut Criterion) {
    let mut group = c.benchmark_group("hamming_f32");

    for dim in &[128, 384, 768, 1536, 3072] {
        let a = generate_binary_vector(*dim, 0);
        let b = generate_binary_vector(*dim, 1);

        group.bench_with_input(BenchmarkId::new("auto_vec", dim), dim, |bencher, _| {
            bencher.iter(|| hamming_distance_fast(black_box(&a), black_box(&b)));
        });

        group.bench_with_input(BenchmarkId::new("unrolled", dim), dim, |bencher, _| {
            bencher.iter(|| hamming_distance_simd(black_box(&a), black_box(&b)));
        });
    }

    group.finish();
}

fn bench_hamming_binary(c: &mut Criterion) {
    let mut group = c.benchmark_group("hamming_binary_u64");

    // Compare f32 Hamming vs packed binary with POPCNT
    // 768 f32 elements = 768 bits = 12 u64s
    for (f32_dim, u64_count) in &[(128, 2), (384, 6), (768, 12), (1536, 24), (3072, 48)] {
        let a_f32 = generate_binary_vector(*f32_dim, 0);
        let b_f32 = generate_binary_vector(*f32_dim, 1);
        let a_u64 = generate_packed_binary(*u64_count, 0x1234_5678);
        let b_u64 = generate_packed_binary(*u64_count, 0x8765_4321);

        group.bench_with_input(
            BenchmarkId::new("f32_baseline", f32_dim),
            f32_dim,
            |bencher, _| {
                bencher.iter(|| hamming_distance_fast(black_box(&a_f32), black_box(&b_f32)));
            },
        );

        group.bench_with_input(
            BenchmarkId::new("u64_popcnt", f32_dim),
            f32_dim,
            |bencher, _| {
                bencher.iter(|| hamming_distance_binary(black_box(&a_u64), black_box(&b_u64)));
            },
        );

        group.bench_with_input(
            BenchmarkId::new("u64_popcnt_fast", f32_dim),
            f32_dim,
            |bencher, _| {
                bencher.iter(|| hamming_distance_binary_fast(black_box(&a_u64), black_box(&b_u64)));
            },
        );
    }

    group.finish();
}

/// Generate set-like vectors for Jaccard similarity benchmarks.
/// Values > 0.5 are considered "in the set".
fn generate_set_vector(dim: usize, density: f32, seed: usize) -> Vec<f32> {
    (0..dim)
        .map(|i| {
            // Use deterministic pseudo-random based on seed and index
            let hash = ((i + seed) as u64).wrapping_mul(0x517c_c1b7_2722_0a95);
            let normalized = (hash as f32) / (u64::MAX as f32);
            if normalized < density {
                1.0
            } else {
                0.0
            }
        })
        .collect()
}

fn bench_jaccard_similarity(c: &mut Criterion) {
    let mut group = c.benchmark_group("jaccard_similarity");

    for dim in &[128, 384, 768, 1536, 3072] {
        // Generate sparse set vectors with ~30% density
        let a = generate_set_vector(*dim, 0.3, 42);
        let b = generate_set_vector(*dim, 0.3, 123);

        group.bench_with_input(BenchmarkId::new("fast", dim), dim, |bencher, _| {
            bencher.iter(|| jaccard_similarity_fast(black_box(&a), black_box(&b)));
        });
    }

    group.finish();
}

fn bench_jaccard_density(c: &mut Criterion) {
    let mut group = c.benchmark_group("jaccard_density");
    let dim = 768;

    // Benchmark different set densities
    for density in &[0.1, 0.3, 0.5, 0.7, 0.9] {
        let a = generate_set_vector(dim, *density, 42);
        let b = generate_set_vector(dim, *density, 123);

        group.bench_with_input(
            BenchmarkId::new("density", format!("{:.0}%", density * 100.0)),
            density,
            |bencher, _| {
                bencher.iter(|| jaccard_similarity_fast(black_box(&a), black_box(&b)));
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_dot_product,
    bench_euclidean_distance,
    bench_cosine_similarity,
    bench_hamming_f32,
    bench_hamming_binary,
    bench_jaccard_similarity,
    bench_jaccard_density
);
criterion_main!(benches);
