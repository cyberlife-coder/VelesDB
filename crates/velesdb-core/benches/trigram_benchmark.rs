//! Benchmarks for Trigram Index (US-CORE-003-03)
//!
//! Validates SOTA 2026 performance targets:
//! - 10K docs: < 5ms
//! - 100K docs: < 20ms
//! - 1M docs: < 100ms

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use velesdb_core::index::trigram::TrigramIndex;

fn generate_documents(count: usize) -> Vec<String> {
    (0..count)
        .map(|i| {
            format!(
                "Document {} with searchable content about {} and related topics",
                i,
                match i % 5 {
                    0 => "technology",
                    1 => "science",
                    2 => "travel",
                    3 => "cooking",
                    _ => "sports",
                }
            )
        })
        .collect()
}

fn bench_trigram_insert(c: &mut Criterion) {
    let mut group = c.benchmark_group("trigram_insert");

    for size in [1_000, 10_000, 100_000] {
        let docs = generate_documents(size);

        group.bench_with_input(BenchmarkId::from_parameter(size), &docs, |b, docs| {
            b.iter(|| {
                let mut index = TrigramIndex::new();
                for (i, doc) in docs.iter().enumerate() {
                    index.insert(i as u64, doc);
                }
                black_box(index)
            });
        });
    }

    group.finish();
}

fn bench_trigram_search(c: &mut Criterion) {
    let mut group = c.benchmark_group("trigram_search");

    for size in [10_000, 100_000] {
        let docs = generate_documents(size);
        let mut index = TrigramIndex::new();
        for (i, doc) in docs.iter().enumerate() {
            index.insert(i as u64, doc);
        }

        group.bench_with_input(BenchmarkId::new("search_like", size), &index, |b, index| {
            b.iter(|| black_box(index.search_like("technology")));
        });

        group.bench_with_input(
            BenchmarkId::new("search_ranked", size),
            &index,
            |b, index| {
                b.iter(|| black_box(index.search_like_ranked("technology", 0.1)));
            },
        );
    }

    group.finish();
}

fn bench_trigram_vs_linear(c: &mut Criterion) {
    let mut group = c.benchmark_group("trigram_vs_linear");

    let docs = generate_documents(10_000);
    let mut index = TrigramIndex::new();
    for (i, doc) in docs.iter().enumerate() {
        index.insert(i as u64, doc);
    }

    // With trigram index
    group.bench_function("with_index_10k", |b| {
        b.iter(|| black_box(index.search_like("technology")));
    });

    // Without index (linear scan simulation)
    group.bench_function("linear_scan_10k", |b| {
        b.iter(|| {
            let pattern = "technology";
            let results: Vec<usize> = docs
                .iter()
                .enumerate()
                .filter(|(_, doc)| doc.contains(pattern))
                .map(|(i, _)| i)
                .collect();
            black_box(results)
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_trigram_insert,
    bench_trigram_search,
    bench_trigram_vs_linear,
);
criterion_main!(benches);
