//! Benchmark HNSW search with Full vs SQ8 vs PQ storage modes.

#![allow(clippy::cast_precision_loss)]

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use tempfile::tempdir;
use velesdb_core::{Collection, DistanceMetric, Point, StorageMode};

fn generate_vector(dimension: usize, seed: usize) -> Vec<f32> {
    (0..dimension)
        .map(|i| {
            let x = ((seed * 31 + i * 17 + 11) % 1000) as f32 / 1000.0;
            x * 2.0 - 1.0
        })
        .collect()
}

fn build_collection(storage_mode: StorageMode, n: usize, dimension: usize) -> Collection {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join(format!("bench_{storage_mode:?}"));
    std::mem::forget(dir); // keep for bench duration

    let collection =
        Collection::create_with_options(path, dimension, DistanceMetric::Euclidean, storage_mode)
            .expect("create collection");

    let points: Vec<Point> = (0..n)
        .map(|id| Point::new(id as u64, generate_vector(dimension, id), None))
        .collect();
    collection.upsert(points).expect("upsert");
    collection
}

fn benchmark_hnsw_latency(c: &mut Criterion) {
    let dimension = 64;
    let n = 2000;
    let query = generate_vector(dimension, 9999);

    let full = build_collection(StorageMode::Full, n, dimension);
    let sq8 = build_collection(StorageMode::SQ8, n, dimension);
    let pq = build_collection(StorageMode::ProductQuantization, n, dimension);

    let mut group = c.benchmark_group("hnsw_latency_full_sq8_pq");
    group.bench_function("full", |b| {
        b.iter(|| full.search_ids(black_box(&query), black_box(20)));
    });
    group.bench_function("sq8", |b| {
        b.iter(|| sq8.search_ids(black_box(&query), black_box(20)));
    });
    group.bench_function("pq", |b| {
        b.iter(|| pq.search_ids(black_box(&query), black_box(20)));
    });
    group.finish();
}

fn benchmark_hnsw_recall(c: &mut Criterion) {
    let dimension = 64;
    let n = 2000;
    let query = generate_vector(dimension, 4242);

    let full = build_collection(StorageMode::Full, n, dimension);
    let sq8 = build_collection(StorageMode::SQ8, n, dimension);
    let pq = build_collection(StorageMode::ProductQuantization, n, dimension);

    let full_top: std::collections::HashSet<u64> = full
        .search_ids(&query, 50)
        .expect("full search")
        .into_iter()
        .map(|(id, _)| id)
        .collect();

    let sq8_recall = sq8
        .search_ids(&query, 50)
        .expect("sq8 search")
        .into_iter()
        .map(|(id, _)| id)
        .filter(|id| full_top.contains(id))
        .count() as f32
        / 50.0;

    let pq_recall = pq
        .search_ids(&query, 50)
        .expect("pq search")
        .into_iter()
        .map(|(id, _)| id)
        .filter(|id| full_top.contains(id))
        .count() as f32
        / 50.0;

    c.bench_function("hnsw_recall_report_full_sq8_pq", |b| {
        b.iter(|| black_box((sq8_recall, pq_recall)));
    });

    println!("Recall@50 vs full: SQ8={sq8_recall:.3}, PQ={pq_recall:.3}");
}

criterion_group!(benches, benchmark_hnsw_latency, benchmark_hnsw_recall);
criterion_main!(benches);
