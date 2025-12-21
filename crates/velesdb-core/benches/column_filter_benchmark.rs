//! Benchmark comparing JSON filtering vs Column Store filtering.
//!
//! Run with: `cargo bench --bench column_filter_benchmark`
//!
//! # WIS-46: Column Store Performance Validation

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use serde_json::{json, Value};
use velesdb_core::column_store::{ColumnStore, ColumnType, ColumnValue};
use velesdb_core::filter::{Condition, Filter};

/// Generate JSON payloads for benchmarking.
fn generate_json_payloads(count: usize) -> Vec<Value> {
    let categories = ["tech", "science", "art", "sports", "music"];
    (0..count)
        .map(|i| {
            json!({
                "category": categories[i % categories.len()],
                "price": (i % 1000) as i64,
                "rating": (i % 50) as f64 / 10.0,
                "active": i % 2 == 0
            })
        })
        .collect()
}

/// Generate a column store with equivalent data.
fn generate_column_store(count: usize) -> ColumnStore {
    let categories = ["tech", "science", "art", "sports", "music"];

    let mut store = ColumnStore::with_schema(&[
        ("category", ColumnType::String),
        ("price", ColumnType::Int),
        ("rating", ColumnType::Float),
        ("active", ColumnType::Bool),
    ]);

    // Pre-intern all categories
    let cat_ids: Vec<_> = categories
        .iter()
        .map(|c| store.string_table_mut().intern(c))
        .collect();

    for i in 0..count {
        store.push_row(&[
            (
                "category",
                ColumnValue::String(cat_ids[i % categories.len()]),
            ),
            ("price", ColumnValue::Int((i % 1000) as i64)),
            ("rating", ColumnValue::Float((i % 50) as f64 / 10.0)),
            ("active", ColumnValue::Bool(i % 2 == 0)),
        ]);
    }

    store
}

// =============================================================================
// EQUALITY FILTER BENCHMARKS
// =============================================================================

fn bench_filter_eq_string(c: &mut Criterion) {
    let mut group = c.benchmark_group("filter_eq_string");

    for count in [1_000, 10_000, 100_000].iter() {
        let payloads = generate_json_payloads(*count);
        let store = generate_column_store(*count);
        let filter = Filter::new(Condition::eq("category", "tech"));

        group.bench_with_input(BenchmarkId::new("json", count), count, |bencher, _| {
            bencher.iter(|| {
                let matches: Vec<_> = payloads
                    .iter()
                    .enumerate()
                    .filter(|(_, p)| filter.matches(black_box(p)))
                    .map(|(i, _)| i)
                    .collect();
                black_box(matches)
            });
        });

        group.bench_with_input(
            BenchmarkId::new("column_store", count),
            count,
            |bencher, _| {
                bencher.iter(|| {
                    let matches = store.filter_eq_string(black_box("category"), black_box("tech"));
                    black_box(matches)
                });
            },
        );
    }

    group.finish();
}

fn bench_filter_eq_int(c: &mut Criterion) {
    let mut group = c.benchmark_group("filter_eq_int");

    for count in [1_000, 10_000, 100_000].iter() {
        let payloads = generate_json_payloads(*count);
        let store = generate_column_store(*count);
        let filter = Filter::new(Condition::eq("price", 500));

        group.bench_with_input(BenchmarkId::new("json", count), count, |bencher, _| {
            bencher.iter(|| {
                let matches: Vec<_> = payloads
                    .iter()
                    .enumerate()
                    .filter(|(_, p)| filter.matches(black_box(p)))
                    .map(|(i, _)| i)
                    .collect();
                black_box(matches)
            });
        });

        group.bench_with_input(
            BenchmarkId::new("column_store", count),
            count,
            |bencher, _| {
                bencher.iter(|| {
                    let matches = store.filter_eq_int(black_box("price"), black_box(500));
                    black_box(matches)
                });
            },
        );
    }

    group.finish();
}

// =============================================================================
// RANGE FILTER BENCHMARKS
// =============================================================================

fn bench_filter_range_int(c: &mut Criterion) {
    let mut group = c.benchmark_group("filter_range_int");

    for count in [1_000, 10_000, 100_000].iter() {
        let payloads = generate_json_payloads(*count);
        let store = generate_column_store(*count);

        // JSON: price > 100 AND price < 500
        let filter = Filter::new(Condition::and(vec![
            Condition::gt("price", 100),
            Condition::lt("price", 500),
        ]));

        group.bench_with_input(BenchmarkId::new("json", count), count, |bencher, _| {
            bencher.iter(|| {
                let matches: Vec<_> = payloads
                    .iter()
                    .enumerate()
                    .filter(|(_, p)| filter.matches(black_box(p)))
                    .map(|(i, _)| i)
                    .collect();
                black_box(matches)
            });
        });

        group.bench_with_input(
            BenchmarkId::new("column_store", count),
            count,
            |bencher, _| {
                bencher.iter(|| {
                    let matches =
                        store.filter_range_int(black_box("price"), black_box(100), black_box(500));
                    black_box(matches)
                });
            },
        );
    }

    group.finish();
}

// =============================================================================
// IN FILTER BENCHMARKS
// =============================================================================

fn bench_filter_in_string(c: &mut Criterion) {
    let mut group = c.benchmark_group("filter_in_string");

    for count in [1_000, 10_000, 100_000].iter() {
        let payloads = generate_json_payloads(*count);
        let store = generate_column_store(*count);

        let filter = Filter::new(Condition::is_in(
            "category",
            vec![json!("tech"), json!("science")],
        ));

        group.bench_with_input(BenchmarkId::new("json", count), count, |bencher, _| {
            bencher.iter(|| {
                let matches: Vec<_> = payloads
                    .iter()
                    .enumerate()
                    .filter(|(_, p)| filter.matches(black_box(p)))
                    .map(|(i, _)| i)
                    .collect();
                black_box(matches)
            });
        });

        group.bench_with_input(
            BenchmarkId::new("column_store", count),
            count,
            |bencher, _| {
                bencher.iter(|| {
                    let matches = store
                        .filter_in_string(black_box("category"), black_box(&["tech", "science"]));
                    black_box(matches)
                });
            },
        );
    }

    group.finish();
}

// =============================================================================
// THROUGHPUT COMPARISON
// =============================================================================

fn bench_throughput_scaling(c: &mut Criterion) {
    let mut group = c.benchmark_group("throughput_scaling");
    group.sample_size(50);

    for count in [1_000, 10_000, 100_000, 500_000].iter() {
        let payloads = generate_json_payloads(*count);
        let store = generate_column_store(*count);
        let filter = Filter::new(Condition::eq("category", "tech"));

        group.bench_with_input(BenchmarkId::new("json", count), count, |bencher, _| {
            bencher.iter(|| {
                payloads
                    .iter()
                    .filter(|p| filter.matches(black_box(p)))
                    .count()
            });
        });

        group.bench_with_input(
            BenchmarkId::new("column_store", count),
            count,
            |bencher, _| {
                bencher.iter(|| store.count_eq_string(black_box("category"), black_box("tech")));
            },
        );
    }

    group.finish();
}

fn bench_bitmap_scaling(c: &mut Criterion) {
    let mut group = c.benchmark_group("bitmap_scaling");
    group.sample_size(50);

    for count in [10_000, 100_000, 500_000].iter() {
        let store = generate_column_store(*count);

        group.bench_with_input(
            BenchmarkId::new("filter_vec", count),
            count,
            |bencher, _| {
                bencher.iter(|| {
                    let matches = store.filter_eq_string(black_box("category"), black_box("tech"));
                    black_box(matches.len())
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("filter_bitmap", count),
            count,
            |bencher, _| {
                bencher.iter(|| {
                    let bitmap =
                        store.filter_eq_string_bitmap(black_box("category"), black_box("tech"));
                    black_box(bitmap.len())
                });
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_filter_eq_string,
    bench_filter_eq_int,
    bench_filter_range_int,
    bench_filter_in_string,
    bench_throughput_scaling,
    bench_bitmap_scaling
);
criterion_main!(benches);
