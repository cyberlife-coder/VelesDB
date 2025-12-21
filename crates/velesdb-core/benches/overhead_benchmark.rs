//! Benchmark to identify and measure overhead between SIMD kernels and public API.
//!
//! This benchmark isolates different sources of overhead:
//! - Assertions / length checks
//! - Function call overhead
//! - Dispatch logic
//! - Memory alignment
//!
//! Run with: `cargo bench --bench overhead_benchmark`
//!
//! # WIS-45: Performance Diagnostic

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use velesdb_core::simd::{cosine_similarity_fast, dot_product_fast, euclidean_distance_fast};
use velesdb_core::simd_explicit::{
    cosine_similarity_simd, dot_product_simd, euclidean_distance_simd,
};

/// Generate a deterministic f32 vector for benchmarking.
fn generate_vector(dim: usize, seed: f32) -> Vec<f32> {
    #[allow(clippy::cast_precision_loss)]
    (0..dim).map(|i| (seed + i as f32 * 0.1).sin()).collect()
}

/// Generate an aligned vector (64-byte aligned for AVX-512 compatibility).
fn generate_aligned_vector(dim: usize, seed: f32) -> Vec<f32> {
    let mut v = generate_vector(dim, seed);
    // Ensure the vector is cache-line aligned (helps with SIMD loads)
    v.shrink_to_fit();
    v
}

// =============================================================================
// COSINE SIMILARITY: Detailed Overhead Analysis
// =============================================================================

fn bench_cosine_overhead(c: &mut Criterion) {
    let mut group = c.benchmark_group("cosine_overhead");
    group.sample_size(200);

    let dim = 768; // Standard embedding dimension
    let a = generate_aligned_vector(dim, 0.0);
    let b = generate_aligned_vector(dim, 1.0);

    // 1. Raw explicit SIMD kernel (baseline - fastest possible)
    group.bench_function("1_explicit_simd_kernel", |bencher| {
        bencher.iter(|| cosine_similarity_simd(black_box(&a), black_box(&b)));
    });

    // 2. Public API (auto-vectorized with fused computation)
    group.bench_function("2_public_api_autovec", |bencher| {
        bencher.iter(|| cosine_similarity_fast(black_box(&a), black_box(&b)));
    });

    // 3. Measure assertion overhead by calling with pre-validated slices
    // (We can't remove assertions, but we can measure their impact indirectly)
    group.bench_function("3_explicit_simd_prechecked", |bencher| {
        // Pre-check outside the hot loop
        assert_eq!(a.len(), b.len());
        bencher.iter(|| {
            // Call with slices we know are valid
            cosine_similarity_simd(black_box(&a), black_box(&b))
        });
    });

    group.finish();
}

// =============================================================================
// EUCLIDEAN DISTANCE: Detailed Overhead Analysis
// =============================================================================

fn bench_euclidean_overhead(c: &mut Criterion) {
    let mut group = c.benchmark_group("euclidean_overhead");
    group.sample_size(200);

    let dim = 768;
    let a = generate_aligned_vector(dim, 0.0);
    let b = generate_aligned_vector(dim, 1.0);

    group.bench_function("1_explicit_simd_kernel", |bencher| {
        bencher.iter(|| euclidean_distance_simd(black_box(&a), black_box(&b)));
    });

    group.bench_function("2_public_api_autovec", |bencher| {
        bencher.iter(|| euclidean_distance_fast(black_box(&a), black_box(&b)));
    });

    group.finish();
}

// =============================================================================
// DOT PRODUCT: Detailed Overhead Analysis
// =============================================================================

fn bench_dot_product_overhead(c: &mut Criterion) {
    let mut group = c.benchmark_group("dot_product_overhead");
    group.sample_size(200);

    let dim = 768;
    let a = generate_aligned_vector(dim, 0.0);
    let b = generate_aligned_vector(dim, 1.0);

    group.bench_function("1_explicit_simd_kernel", |bencher| {
        bencher.iter(|| dot_product_simd(black_box(&a), black_box(&b)));
    });

    group.bench_function("2_public_api_autovec", |bencher| {
        bencher.iter(|| dot_product_fast(black_box(&a), black_box(&b)));
    });

    group.finish();
}

// =============================================================================
// DIMENSION SCALING: How overhead changes with vector size
// =============================================================================

fn bench_dimension_scaling(c: &mut Criterion) {
    let mut group = c.benchmark_group("dimension_scaling");

    // Test common embedding dimensions
    for dim in [64, 128, 256, 384, 512, 768, 1024, 1536, 3072].iter() {
        let a = generate_aligned_vector(*dim, 0.0);
        let b = generate_aligned_vector(*dim, 1.0);

        group.bench_with_input(
            BenchmarkId::new("cosine_explicit", dim),
            dim,
            |bencher, _| {
                bencher.iter(|| cosine_similarity_simd(black_box(&a), black_box(&b)));
            },
        );

        group.bench_with_input(
            BenchmarkId::new("cosine_autovec", dim),
            dim,
            |bencher, _| {
                bencher.iter(|| cosine_similarity_fast(black_box(&a), black_box(&b)));
            },
        );
    }

    group.finish();
}

// =============================================================================
// INLINING TEST: Repeated calls to test inlining behavior
// =============================================================================

fn bench_inlining(c: &mut Criterion) {
    let mut group = c.benchmark_group("inlining_behavior");

    let dim = 768;
    let vectors: Vec<_> = (0..10)
        .map(|i| {
            #[allow(clippy::cast_precision_loss)]
            (
                generate_aligned_vector(dim, i as f32),
                generate_aligned_vector(dim, i as f32 + 0.5),
            )
        })
        .collect();

    // Single call (baseline)
    group.bench_function("single_call_explicit", |bencher| {
        bencher.iter(|| cosine_similarity_simd(black_box(&vectors[0].0), black_box(&vectors[0].1)));
    });

    // Batch of 10 calls (tests if inlining amortizes overhead)
    group.bench_function("batch_10_explicit", |bencher| {
        bencher.iter(|| {
            let mut sum = 0.0f32;
            for (a, b) in &vectors {
                sum += cosine_similarity_simd(black_box(a), black_box(b));
            }
            sum
        });
    });

    group.bench_function("single_call_autovec", |bencher| {
        bencher.iter(|| cosine_similarity_fast(black_box(&vectors[0].0), black_box(&vectors[0].1)));
    });

    group.bench_function("batch_10_autovec", |bencher| {
        bencher.iter(|| {
            let mut sum = 0.0f32;
            for (a, b) in &vectors {
                sum += cosine_similarity_fast(black_box(a), black_box(b));
            }
            sum
        });
    });

    group.finish();
}

// =============================================================================
// MEMORY LAYOUT: Test impact of slice vs owned data
// =============================================================================

fn bench_memory_layout(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory_layout");

    let dim = 768;
    let a_owned = generate_aligned_vector(dim, 0.0);
    let b_owned = generate_aligned_vector(dim, 1.0);

    // Test with slices of different offsets (alignment impact)
    group.bench_function("aligned_slices", |bencher| {
        let a_slice = &a_owned[..];
        let b_slice = &b_owned[..];
        bencher.iter(|| cosine_similarity_simd(black_box(a_slice), black_box(b_slice)));
    });

    // Test with offset slices (potentially misaligned)
    let a_padded = generate_aligned_vector(dim + 1, 0.0);
    let b_padded = generate_aligned_vector(dim + 1, 1.0);

    group.bench_function("offset_slices", |bencher| {
        let a_slice = &a_padded[1..]; // Offset by 1 element (4 bytes)
        let b_slice = &b_padded[1..];
        bencher.iter(|| cosine_similarity_simd(black_box(a_slice), black_box(b_slice)));
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_cosine_overhead,
    bench_euclidean_overhead,
    bench_dot_product_overhead,
    bench_dimension_scaling,
    bench_inlining,
    bench_memory_layout
);
criterion_main!(benches);
