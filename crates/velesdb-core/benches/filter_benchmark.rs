//! Benchmarks for metadata filtering performance.
//!
//! Measures the overhead of filtering operations on search results.

#![allow(clippy::cast_precision_loss)]

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use serde_json::{json, Value};
use velesdb_core::filter::{Condition, Filter};

/// Generate test payloads with various field types.
fn generate_payloads(count: usize) -> Vec<Value> {
    (0..count)
        .map(|i| {
            json!({
                "id": i,
                "category": if i % 3 == 0 { "tech" } else if i % 3 == 1 { "science" } else { "art" },
                "price": (i % 1000) as f64 + 0.99,
                "rating": (i % 50) as f64 / 10.0,
                "active": i % 2 == 0,
                "tags": ["tag1", "tag2", "tag3"],
                "metadata": {
                    "author": format!("author_{}", i % 100),
                    "year": 2020 + (i % 5),
                    "views": i * 10
                },
                "title": format!("Document {} about various topics including rust and performance", i)
            })
        })
        .collect()
}

/// Benchmark simple equality filter.
fn bench_filter_equality(c: &mut Criterion) {
    let payloads = generate_payloads(10_000);
    let filter = Filter::new(Condition::eq("category", "tech"));

    c.bench_function("filter_equality_10k", |b| {
        b.iter(|| {
            let count: usize = payloads
                .iter()
                .filter(|p| filter.matches(black_box(p)))
                .count();
            black_box(count)
        });
    });
}

/// Benchmark range filter (gt, lt).
fn bench_filter_range(c: &mut Criterion) {
    let payloads = generate_payloads(10_000);
    let filter = Filter::new(Condition::and(vec![
        Condition::gt("price", 100.0),
        Condition::lt("price", 500.0),
    ]));

    c.bench_function("filter_range_10k", |b| {
        b.iter(|| {
            let count: usize = payloads
                .iter()
                .filter(|p| filter.matches(black_box(p)))
                .count();
            black_box(count)
        });
    });
}

/// Benchmark IN filter.
fn bench_filter_in(c: &mut Criterion) {
    let payloads = generate_payloads(10_000);
    let filter = Filter::new(Condition::is_in(
        "category",
        vec![json!("tech"), json!("science")],
    ));

    c.bench_function("filter_in_10k", |b| {
        b.iter(|| {
            let count: usize = payloads
                .iter()
                .filter(|p| filter.matches(black_box(p)))
                .count();
            black_box(count)
        });
    });
}

/// Benchmark nested field access.
fn bench_filter_nested(c: &mut Criterion) {
    let payloads = generate_payloads(10_000);
    let filter = Filter::new(Condition::gt("metadata.views", 5000));

    c.bench_function("filter_nested_10k", |b| {
        b.iter(|| {
            let count: usize = payloads
                .iter()
                .filter(|p| filter.matches(black_box(p)))
                .count();
            black_box(count)
        });
    });
}

/// Benchmark contains (string search).
fn bench_filter_contains(c: &mut Criterion) {
    let payloads = generate_payloads(10_000);
    let filter = Filter::new(Condition::contains("title", "rust"));

    c.bench_function("filter_contains_10k", |b| {
        b.iter(|| {
            let count: usize = payloads
                .iter()
                .filter(|p| filter.matches(black_box(p)))
                .count();
            black_box(count)
        });
    });
}

/// Benchmark complex combined filter (AND + OR).
fn bench_filter_complex(c: &mut Criterion) {
    let payloads = generate_payloads(10_000);

    // (category = "tech" AND price > 100) OR (rating >= 4.0 AND active = true)
    let filter = Filter::new(Condition::or(vec![
        Condition::and(vec![
            Condition::eq("category", "tech"),
            Condition::gt("price", 100.0),
        ]),
        Condition::and(vec![
            Condition::gte("rating", 4.0),
            Condition::eq("active", true),
        ]),
    ]));

    c.bench_function("filter_complex_10k", |b| {
        b.iter(|| {
            let count: usize = payloads
                .iter()
                .filter(|p| filter.matches(black_box(p)))
                .count();
            black_box(count)
        });
    });
}

/// Benchmark filter throughput at different scales.
fn bench_filter_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("filter_throughput");

    for size in [1_000, 10_000, 100_000] {
        let payloads = generate_payloads(size);
        let filter = Filter::new(Condition::eq("category", "tech"));

        group.throughput(Throughput::Elements(size as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(size),
            &payloads,
            |b, payloads| {
                b.iter(|| {
                    let count: usize = payloads
                        .iter()
                        .filter(|p| filter.matches(black_box(p)))
                        .count();
                    black_box(count)
                });
            },
        );
    }

    group.finish();
}

/// Benchmark filter selectivity impact.
fn bench_filter_selectivity(c: &mut Criterion) {
    let mut group = c.benchmark_group("filter_selectivity");
    let payloads = generate_payloads(10_000);

    // High selectivity (~33% match)
    let filter_high = Filter::new(Condition::eq("category", "tech"));

    // Low selectivity (~1% match)
    let filter_low = Filter::new(Condition::and(vec![
        Condition::eq("category", "tech"),
        Condition::gt("price", 900.0),
    ]));

    group.bench_function("high_selectivity_33pct", |b| {
        b.iter(|| {
            let count: usize = payloads
                .iter()
                .filter(|p| filter_high.matches(black_box(p)))
                .count();
            black_box(count)
        });
    });

    group.bench_function("low_selectivity_1pct", |b| {
        b.iter(|| {
            let count: usize = payloads
                .iter()
                .filter(|p| filter_low.matches(black_box(p)))
                .count();
            black_box(count)
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_filter_equality,
    bench_filter_range,
    bench_filter_in,
    bench_filter_nested,
    bench_filter_contains,
    bench_filter_complex,
    bench_filter_throughput,
    bench_filter_selectivity,
);

criterion_main!(benches);
