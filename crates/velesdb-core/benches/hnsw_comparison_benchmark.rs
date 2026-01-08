//! Benchmark comparing Native HNSW vs hnsw_rs performance.
//!
//! Run with: cargo bench --bench hnsw_comparison_benchmark

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use velesdb_core::index::hnsw::native::{NativeHnsw, SimdDistance};
use velesdb_core::index::hnsw::{HnswIndex, HnswParams};
use velesdb_core::index::VectorIndex;
use velesdb_core::DistanceMetric;

const DIMENSIONS: usize = 128;
const N_VECTORS: usize = 5000;
const K: usize = 10;
const EF_SEARCH: usize = 128;

fn generate_vectors(n: usize, dim: usize) -> Vec<Vec<f32>> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    (0..n)
        .map(|i| {
            (0..dim)
                .map(|j| {
                    let mut hasher = DefaultHasher::new();
                    (i * dim + j).hash(&mut hasher);
                    (hasher.finish() as f32 / u64::MAX as f32) * 2.0 - 1.0
                })
                .collect()
        })
        .collect()
}

fn bench_insert(c: &mut Criterion) {
    let vectors = generate_vectors(N_VECTORS, DIMENSIONS);
    let mut group = c.benchmark_group("hnsw_insert");

    group.bench_function("native_hnsw", |b| {
        b.iter(|| {
            let distance = SimdDistance::new(DistanceMetric::Euclidean);
            let hnsw = NativeHnsw::new(distance, 16, 200, N_VECTORS);
            for (i, v) in vectors.iter().enumerate() {
                hnsw.insert(v.clone());
                black_box(i);
            }
            black_box(&hnsw);
        });
    });

    group.bench_function("hnsw_rs", |b| {
        b.iter(|| {
            let index = HnswIndex::with_params(
                DIMENSIONS,
                DistanceMetric::Euclidean,
                HnswParams::custom(16, 200, N_VECTORS),
            );
            for (i, v) in vectors.iter().enumerate() {
                index.insert(i as u64, v);
            }
            black_box(&index);
        });
    });

    group.finish();
}

fn bench_search(c: &mut Criterion) {
    let vectors = generate_vectors(N_VECTORS, DIMENSIONS);
    let queries = generate_vectors(100, DIMENSIONS);

    // Build native index
    let native_distance = SimdDistance::new(DistanceMetric::Euclidean);
    let native_hnsw = NativeHnsw::new(native_distance, 16, 200, N_VECTORS);
    for v in &vectors {
        native_hnsw.insert(v.clone());
    }

    // Build hnsw_rs index
    let hnsw_rs_index = HnswIndex::with_params(
        DIMENSIONS,
        DistanceMetric::Euclidean,
        HnswParams::custom(16, 200, N_VECTORS),
    );
    for (i, v) in vectors.iter().enumerate() {
        hnsw_rs_index.insert(i as u64, v);
    }

    let mut group = c.benchmark_group("hnsw_search");

    group.bench_function("native_hnsw", |b| {
        b.iter(|| {
            for q in &queries {
                let results = native_hnsw.search_neighbours(q, K, EF_SEARCH);
                black_box(results);
            }
        });
    });

    group.bench_function("hnsw_rs", |b| {
        b.iter(|| {
            for q in &queries {
                let results = hnsw_rs_index.search(q, K);
                black_box(results);
            }
        });
    });

    group.finish();
}

fn bench_parallel_insert(c: &mut Criterion) {
    let vectors: Vec<Vec<f32>> = generate_vectors(N_VECTORS, DIMENSIONS);
    let mut group = c.benchmark_group("hnsw_parallel_insert");

    group.bench_function("native_hnsw_parallel", |b| {
        b.iter(|| {
            let distance = SimdDistance::new(DistanceMetric::Euclidean);
            let hnsw = NativeHnsw::new(distance, 16, 200, N_VECTORS);
            let data: Vec<(&Vec<f32>, usize)> =
                vectors.iter().enumerate().map(|(i, v)| (v, i)).collect();
            hnsw.parallel_insert(&data);
            black_box(&hnsw);
        });
    });

    group.bench_function("hnsw_rs_sequential", |b| {
        b.iter(|| {
            let index = HnswIndex::with_params(
                DIMENSIONS,
                DistanceMetric::Euclidean,
                HnswParams::custom(16, 200, N_VECTORS),
            );
            for (i, v) in vectors.iter().enumerate() {
                index.insert(i as u64, v);
            }
            black_box(&index);
        });
    });

    group.finish();
}

criterion_group!(benches, bench_insert, bench_search, bench_parallel_insert);
criterion_main!(benches);
