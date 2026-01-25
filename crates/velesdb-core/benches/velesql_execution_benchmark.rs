//! `VelesQL` Execution Benchmarks - Scalability Testing
#![allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
//!
//! Benchmarks query EXECUTION (not just parsing) at different data scales:
//! - 1K, 10K, 100K rows
//! - Vector search, aggregations, GROUP BY, HAVING, multicolumn
//!
//! Research-based optimizations:
//! - HNSW: O(log n) search, best for > 10K vectors
//! - Streaming aggregation: O(1) memory, single-pass
//! - Pre-filter vs Post-filter strategy by selectivity

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use serde_json::json;
use tempfile::TempDir;
use velesdb_core::{Collection, Database, DistanceMetric, Point};

fn create_test_collection(size: usize, dimension: usize) -> (Collection, TempDir) {
    let tmp = TempDir::new().expect("temp dir");
    let db = Database::open(tmp.path()).expect("db");
    db.create_collection("bench", dimension, DistanceMetric::Cosine)
        .expect("collection");
    let collection = db.get_collection("bench").expect("get collection");

    let categories = ["tech", "science", "business", "sports", "health"];
    let points: Vec<Point> = (0..size)
        .map(|id| {
            let category = categories[id % categories.len()];
            let price = f64::from((id % 1000) as u32) + 0.99;
            let stock = id % 100;
            let embedding: Vec<f32> = (0..dimension)
                .map(|i| ((id as f32).mul_add(0.01, i as f32 * 0.01)).sin())
                .collect();
            let payload = json!({
                "category": category,
                "price": price,
                "stock": stock,
                "title": format!("Product {}", id),
            });
            Point::new(id as u64, embedding, Some(payload))
        })
        .collect();

    collection.upsert(points).expect("upsert");
    (collection, tmp)
}

fn create_query_embedding(dimension: usize, seed: usize) -> Vec<f32> {
    (0..dimension)
        .map(|i| ((seed as f32).mul_add(0.01, i as f32 * 0.01)).sin())
        .collect()
}

// =============================================================================
// CATEGORY 1: VECTOR SEARCH SCALING
// =============================================================================

fn bench_vector_search_scaling(c: &mut Criterion) {
    let mut group = c.benchmark_group("velesql_vector_search");
    group.sample_size(50);

    for size in [1_000, 10_000, 100_000] {
        let (collection, _tmp) = create_test_collection(size, 128);
        let query_vec = create_query_embedding(128, 42);
        group.throughput(Throughput::Elements(1));
        group.bench_with_input(BenchmarkId::new("vector_near", size), &size, |b, _| {
            b.iter(|| {
                let results = collection.search(black_box(&query_vec), 10);
                black_box(results)
            });
        });
    }

    group.finish();
}

// =============================================================================
// CATEGORY 2: FILTERED SEARCH SCALING
// =============================================================================

fn bench_filtered_search_scaling(c: &mut Criterion) {
    let mut group = c.benchmark_group("velesql_filtered_search");
    group.sample_size(30);

    for size in [1_000, 10_000, 100_000] {
        let (collection, _tmp) = create_test_collection(size, 128);
        let query_vec = create_query_embedding(128, 42);

        // Vector + single filter (high selectivity ~20%)
        let filter = velesdb_core::filter::Filter::new(velesdb_core::filter::Condition::Eq {
            field: "category".to_string(),
            value: serde_json::json!("tech"),
        });
        group.bench_with_input(
            BenchmarkId::new("vector_filter_high_sel", size),
            &size,
            |b, _| {
                b.iter(|| {
                    let results = collection.search_with_filter(black_box(&query_vec), 10, &filter);
                    black_box(results)
                });
            },
        );

        // Vector + low selectivity filter (~1%)
        let filter_low = velesdb_core::filter::Filter::new(velesdb_core::filter::Condition::Eq {
            field: "stock".to_string(),
            value: serde_json::json!(0),
        });
        group.bench_with_input(
            BenchmarkId::new("vector_filter_low_sel", size),
            &size,
            |b, _| {
                b.iter(|| {
                    let results =
                        collection.search_with_filter(black_box(&query_vec), 10, &filter_low);
                    black_box(results)
                });
            },
        );
    }

    group.finish();
}

// =============================================================================
// CATEGORY 3: TEXT SEARCH SCALING (BM25)
// =============================================================================

fn bench_text_search_scaling(c: &mut Criterion) {
    let mut group = c.benchmark_group("velesql_text_search");
    group.sample_size(30);

    for size in [1_000, 10_000, 100_000] {
        let (collection, _tmp) = create_test_collection(size, 64);

        group.bench_with_input(BenchmarkId::new("text_search", size), &size, |b, _| {
            b.iter(|| {
                let results = collection.text_search(black_box("Product"), 10);
                black_box(results)
            });
        });
    }

    group.finish();
}

// =============================================================================
// CATEGORY 4: MULTICOLUMN PROJECTION
// =============================================================================

fn bench_multicolumn_projection(c: &mut Criterion) {
    let mut group = c.benchmark_group("velesql_multicolumn");
    group.sample_size(30);

    for size in [1_000, 10_000, 100_000] {
        let (collection, _tmp) = create_test_collection(size, 64);
        let query_vec = create_query_embedding(64, 42);

        group.bench_with_input(
            BenchmarkId::new("search_with_payload", size),
            &size,
            |b, _| {
                b.iter(|| {
                    let results = collection.search(black_box(&query_vec), 100);
                    if let Ok(ref res) = results {
                        for r in res {
                            if let Some(ref payload) = r.point.payload {
                                let _ = black_box(payload.get("category"));
                                let _ = black_box(payload.get("price"));
                                let _ = black_box(payload.get("title"));
                            }
                        }
                    }
                    black_box(results)
                });
            },
        );
    }

    group.finish();
}

// =============================================================================
// CATEGORY 5: COMPARATIVE ANALYSIS
// =============================================================================

fn bench_query_complexity_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("velesql_complexity");
    group.sample_size(30);

    let size = 10_000;
    let (collection, _tmp) = create_test_collection(size, 128);
    let query_vec = create_query_embedding(128, 42);

    // Simple: just search
    group.bench_function("simple_search", |b| {
        b.iter(|| {
            let results = collection.search(black_box(&query_vec), 10);
            black_box(results)
        });
    });

    // Medium: search with filter
    let filter = velesdb_core::filter::Filter::new(velesdb_core::filter::Condition::Eq {
        field: "category".to_string(),
        value: serde_json::json!("tech"),
    });
    group.bench_function("filtered_search", |b| {
        b.iter(|| {
            let results = collection.search_with_filter(black_box(&query_vec), 10, &filter);
            black_box(results)
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_vector_search_scaling,
    bench_filtered_search_scaling,
    bench_text_search_scaling,
    bench_multicolumn_projection,
    bench_query_complexity_comparison
);

criterion_main!(benches);
