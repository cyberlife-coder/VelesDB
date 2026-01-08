//! Benchmark: NativeHnsw vs DualPrecisionHnsw
//!
//! Compares the original float32 implementation with the new
//! dual-precision (int8 traversal + float32 re-ranking) approach.
//!
//! Run with: `cargo bench --bench dual_precision_benchmark`

#![allow(clippy::cast_precision_loss)]

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use velesdb_core::distance::DistanceMetric;
use velesdb_core::index::hnsw::native::{DualPrecisionHnsw, NativeHnsw, SimdDistance};

fn generate_vector(dim: usize, seed: u64) -> Vec<f32> {
    (0..dim)
        .map(|i| ((seed as f32 * 0.1 + i as f32 * 0.01).sin() + 1.0) / 2.0)
        .collect()
}

fn bench_search_latency(c: &mut Criterion) {
    let mut group = c.benchmark_group("search_latency_comparison");

    let dim = 384; // Common embedding dimension
    let num_vectors = 10_000;
    let k = 10;
    let ef_search = 100;

    // Pre-generate vectors
    let vectors: Vec<Vec<f32>> = (0..num_vectors)
        .map(|i| generate_vector(dim, i as u64))
        .collect();

    let query = generate_vector(dim, 99999);

    // === Build NativeHnsw (baseline) ===
    let engine_native = SimdDistance::new(DistanceMetric::Euclidean);
    let native_hnsw = NativeHnsw::new(engine_native, 32, 200, num_vectors);
    for v in &vectors {
        native_hnsw.insert(v.clone());
    }

    // === Build DualPrecisionHnsw (new) ===
    let engine_dual = SimdDistance::new(DistanceMetric::Euclidean);
    let mut dual_hnsw = DualPrecisionHnsw::new(engine_dual, dim, 32, 200, num_vectors);
    for v in &vectors {
        dual_hnsw.insert(v.clone());
    }
    // Force training if not already done
    dual_hnsw.force_train_quantizer();

    // === Benchmark NativeHnsw ===
    group.bench_with_input(
        BenchmarkId::new("NativeHnsw_float32", format!("{num_vectors}x{dim}d")),
        &(),
        |b, ()| {
            b.iter(|| {
                let results = native_hnsw.search(black_box(&query), k, ef_search);
                black_box(results)
            });
        },
    );

    // === Benchmark DualPrecisionHnsw ===
    group.bench_with_input(
        BenchmarkId::new("DualPrecision_int8", format!("{num_vectors}x{dim}d")),
        &(),
        |b, ()| {
            b.iter(|| {
                let results = dual_hnsw.search(black_box(&query), k, ef_search);
                black_box(results)
            });
        },
    );

    group.finish();
}

fn bench_memory_footprint(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory_comparison");

    let dim = 768; // BERT-like embedding
    let num_vectors = 1000;

    // Pre-generate vectors
    let vectors: Vec<Vec<f32>> = (0..num_vectors)
        .map(|i| generate_vector(dim, i as u64))
        .collect();

    // Calculate theoretical memory usage
    let float32_bytes = num_vectors * dim * 4; // 4 bytes per f32
    let int8_bytes = num_vectors * dim; // 1 byte per u8

    println!("\n=== Memory Footprint Analysis ===");
    println!("Vectors: {num_vectors} x {dim}D");
    println!("Float32 storage: {} KB", float32_bytes / 1024);
    println!("Int8 storage: {} KB", int8_bytes / 1024);
    println!(
        "Reduction: {:.1}x",
        float32_bytes as f64 / int8_bytes as f64
    );

    // Benchmark insertion time (includes quantization overhead)
    group.bench_with_input(
        BenchmarkId::new("insert_native", "1000x768"),
        &(),
        |b, ()| {
            b.iter(|| {
                let engine = SimdDistance::new(DistanceMetric::Euclidean);
                let hnsw = NativeHnsw::new(engine, 32, 200, num_vectors);
                for v in &vectors {
                    hnsw.insert(v.clone());
                }
                black_box(hnsw.len())
            });
        },
    );

    group.bench_with_input(
        BenchmarkId::new("insert_dual_precision", "1000x768"),
        &(),
        |b, ()| {
            b.iter(|| {
                let engine = SimdDistance::new(DistanceMetric::Euclidean);
                let mut hnsw = DualPrecisionHnsw::new(engine, dim, 32, 200, num_vectors);
                for v in &vectors {
                    hnsw.insert(v.clone());
                }
                hnsw.force_train_quantizer();
                black_box(hnsw.len())
            });
        },
    );

    group.finish();
}

fn bench_quantized_distance(c: &mut Criterion) {
    use velesdb_core::index::hnsw::native::ScalarQuantizer;

    let mut group = c.benchmark_group("distance_computation");

    let dim = 768;

    // Generate training data
    let v1: Vec<f32> = (0..dim).map(|i| (i as f32 * 0.01).sin()).collect();
    let v2: Vec<f32> = (0..dim).map(|i| (i as f32 * 0.01).cos()).collect();

    let quantizer = ScalarQuantizer::train(&[&v1, &v2]);
    let q1 = quantizer.quantize(&v1);
    let q2 = quantizer.quantize(&v2);

    // Benchmark float32 distance (baseline)
    group.bench_with_input(BenchmarkId::new("float32_euclidean", dim), &(), |b, ()| {
        b.iter(|| {
            let dist: f32 = v1
                .iter()
                .zip(v2.iter())
                .map(|(a, b)| (a - b).powi(2))
                .sum::<f32>()
                .sqrt();
            black_box(dist)
        });
    });

    // Benchmark int8 quantized distance
    group.bench_with_input(BenchmarkId::new("int8_quantized", dim), &(), |b, ()| {
        b.iter(|| {
            let dist = quantizer.distance_l2_quantized(black_box(&q1), black_box(&q2));
            black_box(dist)
        });
    });

    // Benchmark asymmetric distance (float32 query vs int8 candidate)
    group.bench_with_input(
        BenchmarkId::new("asymmetric_f32_vs_i8", dim),
        &(),
        |b, ()| {
            b.iter(|| {
                let dist = quantizer.distance_l2_asymmetric(black_box(&v1), black_box(&q2));
                black_box(dist)
            });
        },
    );

    group.finish();
}

criterion_group!(
    benches,
    bench_search_latency,
    bench_memory_footprint,
    bench_quantized_distance
);
criterion_main!(benches);
