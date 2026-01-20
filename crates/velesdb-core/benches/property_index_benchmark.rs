//! Benchmark comparing PropertyIndex O(1) lookup vs O(n) scan.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use serde_json::{json, Value};
use std::collections::HashMap;
use velesdb_core::collection::graph::PropertyIndex;

const DATASET_SIZES: &[usize] = &[1_000, 10_000, 100_000];

fn create_test_data(size: usize) -> Vec<(u64, Value)> {
    (0..size as u64)
        .map(|i| (i, json!(format!("user{}@example.com", i))))
        .collect()
}

fn bench_indexed_lookup(c: &mut Criterion) {
    let mut group = c.benchmark_group("property_index_lookup");

    for &size in DATASET_SIZES {
        let data = create_test_data(size);

        // Build indexed version
        let mut index = PropertyIndex::new();
        index.create_index("Person", "email");
        for (id, value) in &data {
            index.insert("Person", "email", value, *id);
        }

        // Lookup middle element
        let lookup_value = json!(format!("user{}@example.com", size / 2));

        group.bench_with_input(
            BenchmarkId::new("indexed", size),
            &(&index, &lookup_value),
            |b, (idx, val)| {
                b.iter(|| {
                    black_box(idx.lookup("Person", "email", val));
                });
            },
        );
    }

    group.finish();
}

fn bench_scan_lookup(c: &mut Criterion) {
    let mut group = c.benchmark_group("property_scan_lookup");

    for &size in DATASET_SIZES {
        let data = create_test_data(size);

        // Build scan version (HashMap simulating node storage)
        let nodes: HashMap<u64, Value> = data.into_iter().collect();

        // Lookup middle element
        let lookup_value = json!(format!("user{}@example.com", size / 2));

        group.bench_with_input(
            BenchmarkId::new("scan", size),
            &(&nodes, &lookup_value),
            |b, (node_map, val)| {
                b.iter(|| {
                    // O(n) scan through all nodes
                    let _results: Vec<u64> = node_map
                        .iter()
                        .filter(|(_, v)| v == val)
                        .map(|(id, _)| *id)
                        .collect();
                    black_box(_results);
                });
            },
        );
    }

    group.finish();
}

fn bench_indexed_vs_scan_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("indexed_vs_scan");
    group.sample_size(100);

    let size = 100_000;
    let data = create_test_data(size);

    // Build indexed version
    let mut index = PropertyIndex::new();
    index.create_index("Person", "email");
    for (id, value) in &data {
        index.insert("Person", "email", value, *id);
    }

    // Build scan version
    let nodes: HashMap<u64, Value> = data.into_iter().collect();

    // Lookup middle element
    let lookup_value = json!(format!("user{}@example.com", size / 2));

    group.bench_function("indexed_100k", |b| {
        b.iter(|| {
            black_box(index.lookup("Person", "email", &lookup_value));
        });
    });

    group.bench_function("scan_100k", |b| {
        b.iter(|| {
            let _results: Vec<u64> = nodes
                .iter()
                .filter(|(_, v)| *v == &lookup_value)
                .map(|(id, _)| *id)
                .collect();
            black_box(_results);
        });
    });

    group.finish();
}

fn bench_insert_performance(c: &mut Criterion) {
    let mut group = c.benchmark_group("property_index_insert");

    for &size in &[1_000, 10_000] {
        group.bench_with_input(BenchmarkId::new("insert", size), &size, |b, &sz| {
            b.iter(|| {
                let mut index = PropertyIndex::new();
                index.create_index("Person", "email");
                for i in 0..sz as u64 {
                    let value = json!(format!("user{}@example.com", i));
                    index.insert("Person", "email", &value, i);
                }
                black_box(index);
            });
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_indexed_lookup,
    bench_scan_lookup,
    bench_indexed_vs_scan_comparison,
    bench_insert_performance,
);
criterion_main!(benches);
