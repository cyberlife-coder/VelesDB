//! Criterion benchmarks for Bulk Insert V2 (Issue #488).
//!
//! Compares insertion paths and measures `AsyncIndexBuilder` overhead:
//! - `upsert_bulk` standard path (baseline)
//! - `AsyncIndexBuilder` enqueue + buffer search (buffer overhead)
//! - `AsyncIndexBuilder` enqueue + drain (buffer throughput)

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::uninlined_format_args,
    deprecated
)]

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use velesdb_core::collection::streaming::{AsyncIndexBuilder, AsyncIndexBuilderConfig};
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

/// Generates owned vector tuples for `AsyncIndexBuilder::enqueue`.
fn generate_vector_tuples(count: usize, dim: usize) -> Vec<(u64, Vec<f32>)> {
    (0..count)
        .map(|i| {
            let vector: Vec<f32> = (0..dim)
                .map(|j| ((i * dim + j) % 1000) as f32 / 1000.0)
                .collect();
            (i as u64, vector)
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

/// Benchmark: `AsyncIndexBuilder` enqueue throughput (buffer only, no HNSW).
///
/// Measures the raw enqueue + drain overhead of the async builder buffer,
/// isolating the buffer management cost from HNSW construction.
fn bench_async_builder_enqueue_drain(c: &mut Criterion) {
    let mut group = c.benchmark_group("bulk_insert_v2");
    group.sample_size(10);
    group.throughput(Throughput::Elements(VECTOR_COUNT as u64));

    let tuples = generate_vector_tuples(VECTOR_COUNT, DIMENSION);

    group.bench_function("async_builder_enqueue_drain", |b| {
        b.iter_with_setup(
            || {
                let config = AsyncIndexBuilderConfig {
                    merge_threshold: VECTOR_COUNT + 1,
                    segment_count: Some(4),
                    sync_mode: false,
                };
                (AsyncIndexBuilder::new(config), tuples.clone())
            },
            |(builder, tups)| {
                builder.enqueue(tups);
                let drained = builder.drain_buffer();
                black_box(drained.len());
            },
        );
    });

    group.finish();
}

/// Benchmark: `AsyncIndexBuilder` buffer search latency.
///
/// Measures brute-force search performance in the async builder buffer
/// with 10K buffered vectors, simulating search during deferred indexing.
fn bench_async_builder_buffer_search(c: &mut Criterion) {
    let mut group = c.benchmark_group("bulk_insert_v2");
    group.sample_size(50);

    let tuples = generate_vector_tuples(VECTOR_COUNT, DIMENSION);
    let config = AsyncIndexBuilderConfig {
        merge_threshold: VECTOR_COUNT + 1,
        segment_count: Some(4),
        sync_mode: false,
    };
    let builder = AsyncIndexBuilder::new(config);
    builder.enqueue(tuples);

    let query: Vec<f32> = (0..DIMENSION).map(|j| (j % 1000) as f32 / 1000.0).collect();

    group.bench_function("async_builder_buffer_search_10k", |b| {
        b.iter(|| {
            let results = builder.search_buffer(&query, 10, DistanceMetric::Cosine);
            black_box(results);
        });
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
                    sync_mode: false,
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
    bench_async_builder_enqueue_drain,
    bench_async_builder_buffer_search,
);
criterion_main!(benches);
