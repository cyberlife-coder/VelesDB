//! Benchmark SIMD implementations.
//!
//! Run with: `cargo bench --bench simd_benchmark`

#![allow(clippy::similar_names)]
#![allow(clippy::cast_precision_loss)]

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
#[cfg(feature = "internal-bench")]
use velesdb_core::internal_bench;
#[cfg(feature = "internal-bench")]
use velesdb_core::simd_native::SimdLevel;
use velesdb_core::simd_native::{
    batch_hamming_native, batch_jaccard_native, cosine_similarity_native, dot_product_native,
    euclidean_native, hamming_binary_native, hamming_distance_native, jaccard_similarity_native,
    DistanceEngine,
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

        group.bench_with_input(BenchmarkId::new("dispatch", dim), dim, |bencher, _| {
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

        group.bench_with_input(BenchmarkId::new("dispatch", dim), dim, |bencher, _| {
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

    for dim in &[128, 384, 767, 768, 769, 1536, 3072] {
        let a = generate_vector(*dim, 0.0);
        let b = generate_vector(*dim, 1.0);
        let engine = DistanceEngine::new(*dim);

        group.bench_with_input(BenchmarkId::new("dispatch", dim), dim, |bencher, _| {
            warmup(|| {
                let _ = cosine_similarity_native(&a, &b);
            });
            bencher.iter(|| cosine_similarity_native(black_box(&a), black_box(&b)));
        });

        #[cfg(feature = "internal-bench")]
        group.bench_with_input(BenchmarkId::new("scalar", dim), dim, |bencher, _| {
            warmup(|| {
                let _ = internal_bench::cosine_scalar(&a, &b);
            });
            bencher.iter(|| internal_bench::cosine_scalar(black_box(&a), black_box(&b)));
        });

        group.bench_with_input(BenchmarkId::new("resolved", dim), dim, |bencher, _| {
            warmup(|| {
                let _ = engine.cosine_similarity(&a, &b);
            });
            bencher.iter(|| engine.cosine_similarity(black_box(&a), black_box(&b)));
        });

        #[cfg(feature = "internal-bench")]
        if matches!(
            internal_bench::detected_simd_level(),
            SimdLevel::Avx2 | SimdLevel::Avx512
        ) {
            group.bench_with_input(
                BenchmarkId::new("kernel_avx2_2acc", dim),
                dim,
                |bencher, _| {
                    warmup(|| {
                        let _ = internal_bench::cosine_avx2_2acc(&a, &b);
                    });
                    bencher.iter(|| {
                        internal_bench::cosine_avx2_2acc(black_box(&a), black_box(&b))
                            .expect("AVX2+FMA should be available")
                    });
                },
            );

            group.bench_with_input(
                BenchmarkId::new("kernel_avx2_4acc", dim),
                dim,
                |bencher, _| {
                    warmup(|| {
                        let _ = internal_bench::cosine_avx2_4acc(&a, &b);
                    });
                    bencher.iter(|| {
                        internal_bench::cosine_avx2_4acc(black_box(&a), black_box(&b))
                            .expect("AVX2+FMA should be available")
                    });
                },
            );
        }

        #[cfg(feature = "internal-bench")]
        if matches!(internal_bench::detected_simd_level(), SimdLevel::Avx512) {
            group.bench_with_input(BenchmarkId::new("kernel_avx512", dim), dim, |bencher, _| {
                warmup(|| {
                    let _ = internal_bench::cosine_avx512(&a, &b);
                });
                bencher.iter(|| {
                    internal_bench::cosine_avx512(black_box(&a), black_box(&b))
                        .expect("AVX-512F should be available")
                });
            });
        }
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

        group.bench_with_input(BenchmarkId::new("dispatch", dim), dim, |bencher, _| {
            warmup(|| {
                let _ = hamming_distance_native(&a, &b);
            });
            bencher.iter(|| hamming_distance_native(black_box(&a), black_box(&b)));
        });
    }

    group.finish();
}

fn bench_hamming_binary(c: &mut Criterion) {
    let mut group = c.benchmark_group("hamming_binary_u64");

    for dim_bits in &[512, 1024, 2048, 4096] {
        let words = dim_bits / 64;
        let a: Vec<u64> = (0..words)
            .map(|i: u64| i.wrapping_mul(0x517c_c1b7_2722_0a95))
            .collect();
        let b: Vec<u64> = (0..words)
            .map(|i: u64| (i + 1).wrapping_mul(0x9e37_79b9_7f4a_7c15))
            .collect();

        group.bench_with_input(
            BenchmarkId::new("dispatch", dim_bits),
            dim_bits,
            |bencher, _| {
                bencher.iter(|| hamming_binary_native(black_box(&a), black_box(&b)));
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
            bencher.iter(|| jaccard_similarity_native(black_box(&a), black_box(&b)));
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
                bencher.iter(|| jaccard_similarity_native(black_box(&a), black_box(&b)));
            },
        );
    }

    group.finish();
}

// =============================================================================
// DistanceEngine vs native dispatch comparison
// =============================================================================

fn bench_engine_vs_native_dot_product(c: &mut Criterion) {
    let mut group = c.benchmark_group("engine_vs_native/dot_product");

    for dim in &[128, 384, 768, 1536] {
        let a = generate_vector(*dim, 0.0);
        let b = generate_vector(*dim, 1.0);
        let engine = DistanceEngine::new(*dim);

        group.bench_with_input(BenchmarkId::new("native", dim), dim, |bencher, _| {
            warmup(|| {
                let _ = dot_product_native(&a, &b);
            });
            bencher.iter(|| dot_product_native(black_box(&a), black_box(&b)));
        });

        group.bench_with_input(BenchmarkId::new("engine", dim), dim, |bencher, _| {
            warmup(|| {
                let _ = engine.dot_product(&a, &b);
            });
            bencher.iter(|| engine.dot_product(black_box(&a), black_box(&b)));
        });
    }

    group.finish();
}

fn bench_engine_vs_native_cosine(c: &mut Criterion) {
    let mut group = c.benchmark_group("engine_vs_native/cosine");

    for dim in &[128, 384, 768, 1536] {
        let a = generate_vector(*dim, 0.0);
        let b = generate_vector(*dim, 1.0);
        let engine = DistanceEngine::new(*dim);

        group.bench_with_input(BenchmarkId::new("native", dim), dim, |bencher, _| {
            warmup(|| {
                let _ = cosine_similarity_native(&a, &b);
            });
            bencher.iter(|| cosine_similarity_native(black_box(&a), black_box(&b)));
        });

        group.bench_with_input(BenchmarkId::new("engine", dim), dim, |bencher, _| {
            warmup(|| {
                let _ = engine.cosine_similarity(&a, &b);
            });
            bencher.iter(|| engine.cosine_similarity(black_box(&a), black_box(&b)));
        });
    }

    group.finish();
}

fn bench_engine_batch_simulation(c: &mut Criterion) {
    let mut group = c.benchmark_group("engine_batch_simulation");
    let dim = 768;
    let batch_size = 1000;

    let query = generate_vector(dim, 0.0);
    let candidates: Vec<Vec<f32>> = (0..batch_size)
        .map(|i| generate_vector(dim, i as f32 * 0.01))
        .collect();
    let engine = DistanceEngine::new(dim);

    group.bench_function("native_1000x768", |bencher| {
        bencher.iter(|| {
            let mut sum = 0.0f32;
            for c in &candidates {
                sum += dot_product_native(black_box(c), black_box(&query));
            }
            sum
        });
    });

    group.bench_function("engine_1000x768", |bencher| {
        bencher.iter(|| {
            let mut sum = 0.0f32;
            for c in &candidates {
                sum += engine.dot_product(black_box(c), black_box(&query));
            }
            sum
        });
    });

    group.finish();
}

// =============================================================================
// Batch Hamming & Jaccard benchmarks
// =============================================================================

fn bench_batch_hamming(c: &mut Criterion) {
    let mut group = c.benchmark_group("batch_hamming_f32");

    for dim in &[128, 384, 768, 1536] {
        let query = generate_binary_vector(*dim, 0);
        let candidates: Vec<Vec<f32>> = (0..100)
            .map(|seed| generate_binary_vector(*dim, seed + 10))
            .collect();
        let refs: Vec<&[f32]> = candidates.iter().map(Vec::as_slice).collect();

        group.bench_with_input(BenchmarkId::new("batch_100", dim), dim, |bencher, _| {
            bencher.iter(|| batch_hamming_native(black_box(&refs), black_box(&query)));
        });
    }

    group.finish();
}

fn bench_batch_jaccard(c: &mut Criterion) {
    let mut group = c.benchmark_group("batch_jaccard");

    for dim in &[128, 384, 768, 1536] {
        let query = generate_set_vector(*dim, 0.3, 42);
        let candidates: Vec<Vec<f32>> = (0..100)
            .map(|seed| generate_set_vector(*dim, 0.3, seed + 100))
            .collect();
        let refs: Vec<&[f32]> = candidates.iter().map(Vec::as_slice).collect();

        group.bench_with_input(BenchmarkId::new("batch_100", dim), dim, |bencher, _| {
            bencher.iter(|| batch_jaccard_native(black_box(&refs), black_box(&query)));
        });
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
    bench_jaccard_density,
    bench_engine_vs_native_dot_product,
    bench_engine_vs_native_cosine,
    bench_engine_batch_simulation,
    bench_batch_hamming,
    bench_batch_jaccard,
);
criterion_main!(benches);
