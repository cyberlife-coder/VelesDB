//! Benchmark SIMD implementations.
//!
//! Run with: `cargo bench --bench simd_benchmark`

#![allow(clippy::similar_names)]
#![allow(clippy::cast_precision_loss)]

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use velesdb_core::simd::{
    cosine_similarity_fast, dot_product_fast, euclidean_distance_fast, hamming_distance_fast,
    jaccard_similarity_fast,
};
use velesdb_core::simd_native::{
    cosine_similarity_native, dot_product_native, euclidean_native, hamming_distance_native,
};

fn generate_vector(dim: usize, seed: f32) -> Vec<f32> {
    #[allow(clippy::cast_precision_loss)]
    (0..dim).map(|i| (seed + i as f32 * 0.1).sin()).collect()
}

/// Warmup function to stabilize CPU frequency and caches
fn warmup<F: Fn()>(f: F) {
    for _ in 0..3 {
        f();
    }
}

fn bench_dot_product(c: &mut Criterion) {
    let mut group = c.benchmark_group("dot_product");

    for dim in &[128, 384, 768, 1536, 3072] {
        let a = generate_vector(*dim, 0.0);
        let b = generate_vector(*dim, 1.0);

        group.bench_with_input(BenchmarkId::new("auto_vec", dim), dim, |bencher, _| {
            warmup(|| {
                let _ = dot_product_fast(&a, &b);
            });
            bencher.iter(|| dot_product_fast(black_box(&a), black_box(&b)));
        });

        group.bench_with_input(BenchmarkId::new("simd_native", dim), dim, |bencher, _| {
            warmup(|| {
                let _ = dot_product_native(&a, &b);
            });
            bencher.iter(|| dot_product_native(black_box(&a), black_box(&b)));
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
            warmup(|| {
                let _ = euclidean_distance_fast(&a, &b);
            });
            bencher.iter(|| euclidean_distance_fast(black_box(&a), black_box(&b)));
        });

        group.bench_with_input(BenchmarkId::new("simd_native", dim), dim, |bencher, _| {
            warmup(|| {
                let _ = euclidean_native(&a, &b);
            });
            bencher.iter(|| euclidean_native(black_box(&a), black_box(&b)));
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
            warmup(|| {
                let _ = cosine_similarity_fast(&a, &b);
            });
            bencher.iter(|| cosine_similarity_fast(black_box(&a), black_box(&b)));
        });

        group.bench_with_input(BenchmarkId::new("simd_native", dim), dim, |bencher, _| {
            warmup(|| {
                let _ = cosine_similarity_native(&a, &b);
            });
            bencher.iter(|| cosine_similarity_native(black_box(&a), black_box(&b)));
        });
    }

    group.finish();
}

fn generate_binary_vector(dim: usize, seed: usize) -> Vec<f32> {
    (0..dim)
        .map(|i| if (i + seed) % 3 == 0 { 1.0 } else { 0.0 })
        .collect()
}

// Binary Hamming benchmarks removed - EPIC-075 consolidation
// The simd_native implementation focuses on f32 vectors.
// Binary operations (u64 POPCNT) are handled separately in the distance module.

fn bench_hamming_f32(c: &mut Criterion) {
    let mut group = c.benchmark_group("hamming_f32");

    for dim in &[128, 384, 768, 1536, 3072] {
        let a = generate_binary_vector(*dim, 0);
        let b = generate_binary_vector(*dim, 1);

        group.bench_with_input(BenchmarkId::new("auto_vec", dim), dim, |bencher, _| {
            warmup(|| {
                let _ = hamming_distance_fast(&a, &b);
            });
            bencher.iter(|| hamming_distance_fast(black_box(&a), black_box(&b)));
        });

        group.bench_with_input(BenchmarkId::new("simd_native", dim), dim, |bencher, _| {
            warmup(|| {
                let _ = hamming_distance_native(&a, &b);
            });
            bencher.iter(|| hamming_distance_native(black_box(&a), black_box(&b)));
        });
    }

    group.finish();
}

fn bench_hamming_binary(_c: &mut Criterion) {
    // Binary Hamming benchmarks removed - EPIC-075 consolidation
    // The simd_native implementation focuses on f32 vectors.
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
