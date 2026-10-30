//! Criterion benchmarks for Bulk Insert V2 (Issue #488).
//!
//! Compares insertion paths and measures `AsyncIndexBuilder` overhead:
//! - `upsert_bulk` standard path (baseline)
//! - `upsert_bulk` V2 path (WAL + `DirectVectorWriter` + builder queue)
//! - search while the builder holds 10K points (the exact scan every search
//!   merges until a flush links them)

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::uninlined_format_args,
    deprecated
)]

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use velesdb_core::collection::streaming::AsyncIndexBuilderConfig;
use velesdb_core::distance::DistanceMetric;
use velesdb_core::{Database, Point};

const VECTOR_COUNT: usize = 10_000;
const DIMENSION: usize = 16;
const BATCH_SIZE: usize = 1_000;

/// Generates deterministic points for benchmarking.
fn generate_points(count: usize, dim: usize) -> Vec<Point> {
    (0..count)
        .map(|i| {
            let vector: Vec<f32> = (0..dim)
                .map(|j| ((i * dim + j) % 1000) as f32 / 1000.0)
                .collect();
            Point::without_payload(i as u64, vector)
        })
        .collect()
}

/// Benchmark: standard `upsert_bulk` path (WAL + HNSW synchronous).
fn bench_upsert_bulk_standard(c: &mut Criterion) {
    let mut group = c.benchmark_group("bulk_insert_v2");
    group.sample_size(10);
    group.throughput(Throughput::Elements(VECTOR_COUNT as u64));

    let points = generate_points(VECTOR_COUNT, DIMENSION);

    group.bench_function("upsert_bulk_standard", |b| {
        b.iter_with_setup(
            || {
                let dir = tempfile::tempdir().unwrap();
                let db = Database::open(dir.path()).unwrap();
                db.create_collection("bench", DIMENSION, DistanceMetric::Cosine)
                    .unwrap();
                (dir, db, points.clone())
            },
            |(dir, db, pts)| {
                let col = db.get_vector_collection("bench").unwrap();
                for batch in pts.chunks(BATCH_SIZE) {
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

/// Benchmark: search latency while the builder holds every point.
///
/// `upsert_bulk`'s V2 path returns before the graph links its points, so each
/// search scores everything the builder holds exactly and merges it: this is
/// that cost at 10K pending points, the default `merge_threshold`.
fn bench_search_with_pending(c: &mut Criterion) {
    let mut group = c.benchmark_group("bulk_insert_v2");
    group.sample_size(50);

    let dir = tempfile::tempdir().unwrap();
    let config = AsyncIndexBuilderConfig {
        merge_threshold: VECTOR_COUNT + 1, // the builder holds every point
        segment_count: None,
    };
    let coll = velesdb_core::VectorCollection::create_with_async_builder(
        dir.path().join("bench_pending"),
        DIMENSION,
        DistanceMetric::Cosine,
        config,
    )
    .unwrap();
    for batch in generate_points(VECTOR_COUNT, DIMENSION).chunks(BATCH_SIZE) {
        coll.upsert_bulk(batch).unwrap();
    }

    let query: Vec<f32> = (0..DIMENSION).map(|j| (j % 1000) as f32 / 1000.0).collect();

    group.bench_function("search_with_10k_pending", |b| {
        b.iter(|| black_box(coll.search(&query, 10).unwrap()));
    });

    group.finish();
}

/// Benchmark: V2 wired path — `upsert_bulk` with `AsyncIndexBuilder` configured.
///
/// Creates a collection WITH `async_index_builder` config and measures the
/// full pipeline: WAL + `DirectVectorWriter` + `AsyncIndexBuilder` enqueue.
fn bench_upsert_bulk_v2_wired(c: &mut Criterion) {
    let mut group = c.benchmark_group("bulk_insert_v2");
    group.sample_size(10);
    group.throughput(Throughput::Elements(VECTOR_COUNT as u64));

    let points = generate_points(VECTOR_COUNT, DIMENSION);

    group.bench_function("upsert_bulk_v2_wired", |b| {
        b.iter_with_setup(
            || {
                let dir = tempfile::tempdir().unwrap();
                let config = AsyncIndexBuilderConfig {
                    merge_threshold: VECTOR_COUNT + 1, // Don't trigger flush during bench
                    segment_count: Some(4),
                };
                let coll = velesdb_core::VectorCollection::create_with_async_builder(
                    dir.path().join("bench_v2"),
                    DIMENSION,
                    DistanceMetric::Cosine,
                    config,
                )
                .unwrap();
                (dir, coll, points.clone())
            },
            |(dir, coll, pts)| {
                for batch in pts.chunks(BATCH_SIZE) {
                    coll.upsert_bulk(batch).unwrap();
                }
                black_box(coll.len());
                drop(coll);
                drop(dir);
            },
        );
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_upsert_bulk_standard,
    bench_upsert_bulk_v2_wired,
    bench_search_with_pending,
);
criterion_main!(benches);
