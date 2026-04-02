//! Criterion benchmarks for bitmap pre-filter V2 filtered search.
//!
//! Measures filtered search latency at different selectivity levels
//! to validate the adaptive strategy (full-scan vs HNSW+bitmap).
//!
//! Run with: `cargo bench --bench bitmap_prefilter_benchmark`

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use serde_json::json;
use velesdb_core::{DistanceMetric, Point, StorageMode, VectorCollection};

/// Number of vectors in the benchmark collection.
const NUM_VECTORS: u64 = 10_000;
/// Vector dimensionality.
const DIM: usize = 16;

/// Generates a deterministic vector for a given ID.
fn make_vector(id: u64) -> Vec<f32> {
    let mut v: Vec<f32> = (0..DIM)
        .map(|d| ((id as f32) * 0.13 + (d as f32) * 0.07).cos())
        .collect();
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in &mut v {
            *x /= norm;
        }
    }
    v
}

/// Generates a normalized query vector.
fn query_vector() -> Vec<f32> {
    let mut v: Vec<f32> = (0..DIM).map(|d| (d as f32 * 0.1).sin()).collect();
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in &mut v {
            *x /= norm;
        }
    }
    v
}

/// Creates a benchmark collection with 10K vectors and a secondary index.
///
/// Payload distribution:
/// - 1% `category = "rare"` (IDs 0..100)
/// - 10% `category = "uncommon"` (IDs 100..1000)
/// - 50% `category = "common"` (IDs 1000..5000)
/// - rest `category = "default"`
fn setup_bench_collection(dir: &std::path::Path) -> VectorCollection {
    let collection = VectorCollection::create(
        dir.join("bench_col"),
        "bench_col",
        DIM,
        DistanceMetric::Cosine,
        StorageMode::Full,
    )
    .expect("bench: create collection");

    collection
        .create_index("category")
        .expect("bench: create secondary index");

    let points: Vec<Point> = (0..NUM_VECTORS)
        .map(|id| {
            let category = match id {
                i if i < (NUM_VECTORS / 100) => "rare",
                i if i < (NUM_VECTORS / 10) => "uncommon",
                i if i < (NUM_VECTORS / 2) => "common",
                _ => "default",
            };
            Point::new(id, make_vector(id), Some(json!({ "category": category })))
        })
        .collect();

    collection.upsert(points).expect("bench: upsert");
    collection
}

/// Benchmark: filtered search at 1% selectivity (full-scan path).
fn bench_filtered_search_1pct(c: &mut Criterion) {
    let dir = tempfile::tempdir().expect("bench: temp dir");
    let collection = setup_bench_collection(dir.path());
    let query = query_vector();
    let filter =
        velesdb_core::filter::Filter::new(velesdb_core::filter::Condition::eq("category", "rare"));

    c.bench_function("filtered_search_1pct", |b| {
        b.iter(|| {
            let results = collection
                .search_with_filter(black_box(&query), 10, black_box(&filter))
                .expect("bench: search");
            black_box(results)
        });
    });
}

/// Benchmark: filtered search at 10% selectivity (HNSW+bitmap path).
fn bench_filtered_search_10pct(c: &mut Criterion) {
    let dir = tempfile::tempdir().expect("bench: temp dir");
    let collection = setup_bench_collection(dir.path());
    let query = query_vector();
    let filter = velesdb_core::filter::Filter::new(velesdb_core::filter::Condition::eq(
        "category", "uncommon",
    ));

    c.bench_function("filtered_search_10pct", |b| {
        b.iter(|| {
            let results = collection
                .search_with_filter(black_box(&query), 10, black_box(&filter))
                .expect("bench: search");
            black_box(results)
        });
    });
}

/// Benchmark: filtered search at 50% selectivity (HNSW+bitmap path).
fn bench_filtered_search_50pct(c: &mut Criterion) {
    let dir = tempfile::tempdir().expect("bench: temp dir");
    let collection = setup_bench_collection(dir.path());
    let query = query_vector();
    let filter = velesdb_core::filter::Filter::new(velesdb_core::filter::Condition::eq(
        "category", "common",
    ));

    c.bench_function("filtered_search_50pct", |b| {
        b.iter(|| {
            let results = collection
                .search_with_filter(black_box(&query), 10, black_box(&filter))
                .expect("bench: search");
            black_box(results)
        });
    });
}

/// Benchmark: unfiltered search baseline for comparison.
fn bench_unfiltered_search_baseline(c: &mut Criterion) {
    let dir = tempfile::tempdir().expect("bench: temp dir");
    let collection = setup_bench_collection(dir.path());
    let query = query_vector();

    c.bench_function("unfiltered_search_baseline", |b| {
        b.iter(|| {
            let results = collection
                .search(black_box(&query), 10)
                .expect("bench: search");
            black_box(results)
        });
    });
}

criterion_group!(
    benches,
    bench_filtered_search_1pct,
    bench_filtered_search_10pct,
    bench_filtered_search_50pct,
    bench_unfiltered_search_baseline,
);
criterion_main!(benches);
