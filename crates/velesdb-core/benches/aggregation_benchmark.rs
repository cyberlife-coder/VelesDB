//! Benchmark for parallel aggregation (EPIC-018 US-001).
//!
//! Compares performance at different data scales to prove parallel speedup.

#![allow(clippy::cast_precision_loss, clippy::cast_possible_wrap)]

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use serde_json::json;
use std::collections::HashMap;
use tempfile::TempDir;
use velesdb_core::{Collection, Database, DistanceMetric, Point};

fn create_test_collection(size: usize) -> (Collection, TempDir) {
    let tmp = TempDir::new().expect("temp dir");
    let db = Database::open(tmp.path()).expect("db");
    db.create_collection("bench", 64, DistanceMetric::Cosine)
        .expect("collection");
    let collection = db.get_collection("bench").expect("get collection");

    let categories = ["tech", "science", "business", "sports", "health"];
    let points: Vec<Point> = (0..size)
        .map(|id| {
            let category = categories[id % categories.len()];
            let price = (id % 1000) as f64 + 0.99;
            let stock = (id % 100) as i64;
            let embedding: Vec<f32> = (0..64)
                .map(|i| ((id as f32 * 0.01) + (i as f32 * 0.01)).sin())
                .collect();
            let payload = json!({
                "category": category,
                "price": price,
                "stock": stock,
            });
            Point::new(id as u64, embedding, Some(payload))
        })
        .collect();

    collection.upsert(points).expect("upsert");
    (collection, tmp)
}

fn bench_aggregation_count_star(c: &mut Criterion) {
    let mut group = c.benchmark_group("aggregation_count_star");
    group.sample_size(20);

    for size in [1_000, 10_000, 50_000, 100_000] {
        let (collection, _tmp) = create_test_collection(size);

        let query =
            velesdb_core::velesql::Parser::parse("SELECT COUNT(*) FROM bench").expect("parse");
        let params = HashMap::new();

        group.bench_with_input(BenchmarkId::new("count", size), &size, |b, _| {
            b.iter(|| {
                let result = collection.execute_aggregate(black_box(&query), &params);
                black_box(result)
            });
        });
    }

    group.finish();
}

fn bench_aggregation_sum_avg(c: &mut Criterion) {
    let mut group = c.benchmark_group("aggregation_sum_avg");
    group.sample_size(20);

    for size in [1_000, 10_000, 50_000, 100_000] {
        let (collection, _tmp) = create_test_collection(size);

        let query =
            velesdb_core::velesql::Parser::parse("SELECT SUM(price), AVG(price) FROM bench")
                .expect("parse");
        let params = HashMap::new();

        group.bench_with_input(BenchmarkId::new("sum_avg", size), &size, |b, _| {
            b.iter(|| {
                let result = collection.execute_aggregate(black_box(&query), &params);
                black_box(result)
            });
        });
    }

    group.finish();
}

fn bench_aggregation_min_max(c: &mut Criterion) {
    let mut group = c.benchmark_group("aggregation_min_max");
    group.sample_size(20);

    for size in [1_000, 10_000, 50_000, 100_000] {
        let (collection, _tmp) = create_test_collection(size);

        let query =
            velesdb_core::velesql::Parser::parse("SELECT MIN(price), MAX(price) FROM bench")
                .expect("parse");
        let params = HashMap::new();

        group.bench_with_input(BenchmarkId::new("min_max", size), &size, |b, _| {
            b.iter(|| {
                let result = collection.execute_aggregate(black_box(&query), &params);
                black_box(result)
            });
        });
    }

    group.finish();
}

fn bench_aggregation_groupby(c: &mut Criterion) {
    let mut group = c.benchmark_group("aggregation_groupby");
    group.sample_size(20);

    for size in [1_000, 10_000, 50_000, 100_000] {
        let (collection, _tmp) = create_test_collection(size);

        let query = velesdb_core::velesql::Parser::parse(
            "SELECT category, COUNT(*), SUM(price) FROM bench GROUP BY category",
        )
        .expect("parse");
        let params = HashMap::new();

        group.bench_with_input(BenchmarkId::new("groupby", size), &size, |b, _| {
            b.iter(|| {
                let result = collection.execute_aggregate(black_box(&query), &params);
                black_box(result)
            });
        });
    }

    group.finish();
}

/// Benchmark batch vs sequential processing directly on Aggregator
fn bench_batch_vs_sequential(c: &mut Criterion) {
    use velesdb_core::velesql::Aggregator;

    let mut group = c.benchmark_group("batch_vs_sequential");
    group.sample_size(20);

    for size in [10_000, 50_000, 100_000, 500_000] {
        // Pre-generate values
        let values: Vec<f64> = (0..size).map(|i| f64::from(i) * 0.01 + 1.0).collect();
        let json_values: Vec<serde_json::Value> = values.iter().map(|v| json!(v)).collect();

        // Benchmark sequential (value by value)
        group.bench_with_input(BenchmarkId::new("sequential", size), &size, |b, _| {
            b.iter(|| {
                let mut agg = Aggregator::new();
                for v in &json_values {
                    agg.process_value("price", v);
                }
                black_box(agg.finalize())
            });
        });

        // Benchmark batch processing
        group.bench_with_input(BenchmarkId::new("batch", size), &size, |b, _| {
            b.iter(|| {
                let mut agg = Aggregator::new();
                // Process in batches of 1024
                for chunk in values.chunks(1024) {
                    agg.process_batch("price", chunk);
                }
                black_box(agg.finalize())
            });
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_aggregation_count_star,
    bench_aggregation_sum_avg,
    bench_aggregation_min_max,
    bench_aggregation_groupby,
    bench_batch_vs_sequential
);

criterion_main!(benches);
