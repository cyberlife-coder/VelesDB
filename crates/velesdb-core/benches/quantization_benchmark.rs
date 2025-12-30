//! Benchmarks for SQ8 Scalar Quantization
//!
//! Measures performance of quantized distance functions vs full precision.
//!
//! Run with: `cargo bench --bench quantization_benchmark`

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use velesdb_core::quantization::{
    cosine_similarity_quantized, cosine_similarity_quantized_simd, dot_product_quantized,
    dot_product_quantized_simd, euclidean_squared_quantized, euclidean_squared_quantized_simd,
    QuantizedVector,
};
use velesdb_core::simd_explicit::{
    cosine_similarity_simd, dot_product_simd, euclidean_distance_simd,
};

/// Generate a deterministic vector for benchmarking
fn generate_vector(dimension: usize, seed: usize) -> Vec<f32> {
    (0..dimension)
        .map(|i| {
            let x = ((seed * 7 + i * 13) % 1000) as f32 / 1000.0;
            x * 2.0 - 1.0 // Range [-1, 1]
        })
        .collect()
}

fn bench_quantization_encode(c: &mut Criterion) {
    let mut group = c.benchmark_group("SQ8 Encode");

    for dim in [128, 384, 768, 1536].iter() {
        let vector = generate_vector(*dim, 42);

        group.bench_with_input(BenchmarkId::new("from_f32", dim), dim, |b, _| {
            b.iter(|| QuantizedVector::from_f32(black_box(&vector)))
        });
    }

    group.finish();
}

fn bench_dot_product_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("Dot Product: f32 vs SQ8");

    for dim in [768, 1536].iter() {
        let query = generate_vector(*dim, 1);
        let vector = generate_vector(*dim, 2);
        let quantized = QuantizedVector::from_f32(&vector);

        // f32 baseline (SIMD)
        group.bench_with_input(BenchmarkId::new("f32_simd", dim), dim, |b, _| {
            b.iter(|| dot_product_simd(black_box(&query), black_box(&vector)))
        });

        // SQ8 scalar
        group.bench_with_input(BenchmarkId::new("sq8_scalar", dim), dim, |b, _| {
            b.iter(|| dot_product_quantized(black_box(&query), black_box(&quantized)))
        });

        // SQ8 SIMD
        group.bench_with_input(BenchmarkId::new("sq8_simd", dim), dim, |b, _| {
            b.iter(|| dot_product_quantized_simd(black_box(&query), black_box(&quantized)))
        });
    }

    group.finish();
}

fn bench_euclidean_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("Euclidean: f32 vs SQ8");

    for dim in [768, 1536].iter() {
        let query = generate_vector(*dim, 1);
        let vector = generate_vector(*dim, 2);
        let quantized = QuantizedVector::from_f32(&vector);

        // f32 baseline
        group.bench_with_input(BenchmarkId::new("f32_simd", dim), dim, |b, _| {
            b.iter(|| euclidean_distance_simd(black_box(&query), black_box(&vector)))
        });

        // SQ8 scalar
        group.bench_with_input(BenchmarkId::new("sq8_scalar", dim), dim, |b, _| {
            b.iter(|| euclidean_squared_quantized(black_box(&query), black_box(&quantized)))
        });

        // SQ8 SIMD
        group.bench_with_input(BenchmarkId::new("sq8_simd", dim), dim, |b, _| {
            b.iter(|| euclidean_squared_quantized_simd(black_box(&query), black_box(&quantized)))
        });
    }

    group.finish();
}

fn bench_cosine_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("Cosine: f32 vs SQ8");

    for dim in [768, 1536].iter() {
        let query = generate_vector(*dim, 1);
        let vector = generate_vector(*dim, 2);
        let quantized = QuantizedVector::from_f32(&vector);

        // f32 baseline
        group.bench_with_input(BenchmarkId::new("f32_simd", dim), dim, |b, _| {
            b.iter(|| cosine_similarity_simd(black_box(&query), black_box(&vector)))
        });

        // SQ8 scalar
        group.bench_with_input(BenchmarkId::new("sq8_scalar", dim), dim, |b, _| {
            b.iter(|| cosine_similarity_quantized(black_box(&query), black_box(&quantized)))
        });

        // SQ8 SIMD
        group.bench_with_input(BenchmarkId::new("sq8_simd", dim), dim, |b, _| {
            b.iter(|| cosine_similarity_quantized_simd(black_box(&query), black_box(&quantized)))
        });
    }

    group.finish();
}

fn bench_memory_usage(c: &mut Criterion) {
    let mut group = c.benchmark_group("Memory Usage");

    for dim in [768, 1536].iter() {
        let vector = generate_vector(*dim, 42);
        let quantized = QuantizedVector::from_f32(&vector);

        let f32_bytes = vector.len() * 4;
        let sq8_bytes = quantized.memory_size();
        let ratio = f32_bytes as f32 / sq8_bytes as f32;

        println!(
            "Dimension {}: f32={} bytes, SQ8={} bytes, ratio={:.1}x",
            dim, f32_bytes, sq8_bytes, ratio
        );

        // Benchmark is just to show the stats
        group.bench_with_input(BenchmarkId::new("ratio", dim), dim, |b, _| {
            b.iter(|| black_box(ratio))
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_quantization_encode,
    bench_dot_product_comparison,
    bench_euclidean_comparison,
    bench_cosine_comparison,
    bench_memory_usage,
);
criterion_main!(benches);
