//! Benchmark for bulk import operations
//!
//! Tests insertion throughput with different batch sizes and data formats.

#![allow(
    clippy::uninlined_format_args,
    clippy::cast_precision_loss,
    clippy::doc_markdown
)]

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use velesdb_core::{Database, DistanceMetric, Point};

/// Generate random vectors for testing
fn generate_vectors(count: usize, dimension: usize) -> Vec<Vec<f32>> {
    (0..count)
        .map(|i| {
            (0..dimension)
                .map(|j| ((i * dimension + j) % 1000) as f32 / 1000.0)
                .collect()
        })
        .collect()
}

/// Generate points with payloads
fn generate_points(count: usize, dimension: usize) -> Vec<Point> {
    generate_vectors(count, dimension)
        .into_iter()
        .enumerate()
        .map(|(i, vector)| {
            Point::new(
                i as u64,
                vector,
                Some(serde_json::json!({
                    "title": format!("Document {}", i),
                    "category": if i % 2 == 0 { "tech" } else { "science" }
                })),
            )
        })
        .collect()
}

/// Benchmark single-point insertion
fn bench_single_insert(c: &mut Criterion) {
    let mut group = c.benchmark_group("single_insert");
    group.sample_size(10);

    for dimension in [384, 768] {
        let vectors = generate_vectors(500, dimension);

        group.throughput(Throughput::Elements(500));
        group.bench_with_input(
            BenchmarkId::new("dimension", dimension),
            &dimension,
            |b, &dim| {
                b.iter_with_setup(
                    || {
                        let dir = tempfile::tempdir().unwrap();
                        let db = Database::open(dir.path()).unwrap();
                        db.create_collection("test", dim, DistanceMetric::Cosine)
                            .unwrap();
                        (dir, db, vectors.clone())
                    },
                    |(dir, db, vecs)| {
                        let col = db.get_collection("test").unwrap();
                        for (i, vec) in vecs.iter().enumerate() {
                            col.upsert(vec![Point::without_payload(i as u64, vec.clone())])
                                .unwrap();
                        }
                        drop(db);
                        drop(dir);
                    },
                );
            },
        );
    }
    group.finish();
}

/// Benchmark batch insertion with different batch sizes
fn bench_batch_insert(c: &mut Criterion) {
    let mut group = c.benchmark_group("batch_insert");
    group.sample_size(10);
    let dimension = 768;
    let total_points = 5_000;

    for batch_size in [100, 500, 1000] {
        let points = generate_points(total_points, dimension);

        group.throughput(Throughput::Elements(total_points as u64));
        group.bench_with_input(
            BenchmarkId::new("batch_size", batch_size),
            &batch_size,
            |b, &bs| {
                b.iter_with_setup(
                    || {
                        let dir = tempfile::tempdir().unwrap();
                        let db = Database::open(dir.path()).unwrap();
                        db.create_collection("test", dimension, DistanceMetric::Cosine)
                            .unwrap();
                        (dir, db, points.clone())
                    },
                    |(dir, db, pts)| {
                        let col = db.get_collection("test").unwrap();
                        for batch in pts.chunks(bs) {
                            col.upsert(batch.to_vec()).unwrap();
                        }
                        black_box(col.len());
                        drop(db);
                        drop(dir);
                    },
                );
            },
        );
    }
    group.finish();
}

/// Benchmark upsert_bulk vs upsert (parallel HNSW insert comparison)
fn bench_bulk_vs_regular(c: &mut Criterion) {
    let mut group = c.benchmark_group("bulk_vs_regular");
    group.sample_size(10);
    let dimension = 768;
    let total_points = 5_000;
    let batch_size = 1000;

    let points = generate_points(total_points, dimension);

    group.throughput(Throughput::Elements(total_points as u64));

    // Regular upsert (sequential HNSW insert)
    group.bench_function("upsert_regular", |b| {
        b.iter_with_setup(
            || {
                let dir = tempfile::tempdir().unwrap();
                let db = Database::open(dir.path()).unwrap();
                db.create_collection("test", dimension, DistanceMetric::Cosine)
                    .unwrap();
                (dir, db, points.clone())
            },
            |(dir, db, pts)| {
                let col = db.get_collection("test").unwrap();
                for batch in pts.chunks(batch_size) {
                    col.upsert(batch.to_vec()).unwrap();
                }
                black_box(col.len());
                drop(db);
                drop(dir);
            },
        );
    });

    // Optimized upsert_bulk (parallel HNSW insert)
    group.bench_function("upsert_bulk", |b| {
        b.iter_with_setup(
            || {
                let dir = tempfile::tempdir().unwrap();
                let db = Database::open(dir.path()).unwrap();
                db.create_collection("test", dimension, DistanceMetric::Cosine)
                    .unwrap();
                (dir, db, points.clone())
            },
            |(dir, db, pts)| {
                let col = db.get_collection("test").unwrap();
                for batch in pts.chunks(batch_size) {
                    col.upsert_bulk(batch).unwrap();
                }
                black_box(col.len());
                drop(db);
                drop(dir);
            },
        );
    });

    group.finish();
}

/// Benchmark insertion throughput at scale
fn bench_insert_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("insert_throughput");
    group.sample_size(10);

    let dimension = 768;
    let batch_size = 1000;

    for count in [1_000, 5_000, 10_000] {
        let points = generate_points(count, dimension);

        group.throughput(Throughput::Elements(count as u64));
        group.bench_with_input(BenchmarkId::new("vectors", count), &count, |b, _| {
            b.iter_with_setup(
                || {
                    let dir = tempfile::tempdir().unwrap();
                    let db = Database::open(dir.path()).unwrap();
                    db.create_collection("test", dimension, DistanceMetric::Cosine)
                        .unwrap();
                    (dir, db, points.clone())
                },
                |(dir, db, pts)| {
                    let col = db.get_collection("test").unwrap();
                    for batch in pts.chunks(batch_size) {
                        col.upsert(batch.to_vec()).unwrap();
                    }
                    black_box(col.len());
                    drop(db);
                    drop(dir);
                },
            );
        });
    }
    group.finish();
}

/// Benchmark with payload vs without payload
fn bench_payload_overhead(c: &mut Criterion) {
    let mut group = c.benchmark_group("payload_overhead");
    group.sample_size(10);
    let dimension = 768;
    let count = 2000;
    let batch_size = 500;

    // Without payload
    let points_no_payload: Vec<Point> = generate_vectors(count, dimension)
        .into_iter()
        .enumerate()
        .map(|(i, v)| Point::without_payload(i as u64, v))
        .collect();

    // With payload
    let points_with_payload = generate_points(count, dimension);

    group.throughput(Throughput::Elements(count as u64));

    group.bench_function("without_payload", |b| {
        b.iter_with_setup(
            || {
                let dir = tempfile::tempdir().unwrap();
                let db = Database::open(dir.path()).unwrap();
                db.create_collection("test", dimension, DistanceMetric::Cosine)
                    .unwrap();
                (dir, db, points_no_payload.clone())
            },
            |(dir, db, pts)| {
                let col = db.get_collection("test").unwrap();
                for batch in pts.chunks(batch_size) {
                    col.upsert(batch.to_vec()).unwrap();
                }
                black_box(col.len());
                drop(db);
                drop(dir);
            },
        );
    });

    group.bench_function("with_payload", |b| {
        b.iter_with_setup(
            || {
                let dir = tempfile::tempdir().unwrap();
                let db = Database::open(dir.path()).unwrap();
                db.create_collection("test", dimension, DistanceMetric::Cosine)
                    .unwrap();
                (dir, db, points_with_payload.clone())
            },
            |(dir, db, pts)| {
                let col = db.get_collection("test").unwrap();
                for batch in pts.chunks(batch_size) {
                    col.upsert(batch.to_vec()).unwrap();
                }
                black_box(col.len());
                drop(db);
                drop(dir);
            },
        );
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_single_insert,
    bench_batch_insert,
    bench_bulk_vs_regular,
    bench_insert_throughput,
    bench_payload_overhead,
);
criterion_main!(benches);
