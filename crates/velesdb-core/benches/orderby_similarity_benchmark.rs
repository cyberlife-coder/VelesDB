//! Benchmark for ORDER BY `similarity()` performance (EPIC-008/US-010).
//!
//! Validates AC-3: < 200µs for sorting by similarity score.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use velesdb_core::velesql::Parser;

/// ORDER BY similarity DESC query
const ORDERBY_SIMILARITY_DESC: &str =
    "SELECT * FROM docs WHERE category = 'tech' ORDER BY similarity(embedding, $v) DESC LIMIT 10";

/// ORDER BY similarity ASC query  
const ORDERBY_SIMILARITY_ASC: &str =
    "SELECT * FROM docs ORDER BY similarity(embedding, $v) ASC LIMIT 5";

/// ORDER BY similarity with WHERE similarity
const ORDERBY_WHERE_SIMILARITY: &str =
    "SELECT * FROM docs WHERE similarity(embedding, $v) > 0.5 ORDER BY similarity(embedding, $v) DESC LIMIT 10";

/// Multiple ORDER BY columns
const ORDERBY_MULTIPLE: &str =
    "SELECT * FROM docs ORDER BY similarity(embedding, $v) DESC, created_at ASC LIMIT 10";

/// Benchmark: Parse ORDER BY similarity DESC
/// Target: < 50µs
fn bench_parse_orderby_similarity_desc(c: &mut Criterion) {
    c.bench_function("orderby_parse_similarity_desc", |b| {
        b.iter(|| {
            let result = Parser::parse(black_box(ORDERBY_SIMILARITY_DESC));
            black_box(result)
        });
    });
}

/// Benchmark: Parse ORDER BY similarity ASC
fn bench_parse_orderby_similarity_asc(c: &mut Criterion) {
    c.bench_function("orderby_parse_similarity_asc", |b| {
        b.iter(|| {
            let result = Parser::parse(black_box(ORDERBY_SIMILARITY_ASC));
            black_box(result)
        });
    });
}

/// Benchmark: Parse combined WHERE + ORDER BY similarity
fn bench_parse_where_orderby_similarity(c: &mut Criterion) {
    c.bench_function("orderby_parse_where_orderby", |b| {
        b.iter(|| {
            let result = Parser::parse(black_box(ORDERBY_WHERE_SIMILARITY));
            black_box(result)
        });
    });
}

/// Benchmark: Parse multiple ORDER BY columns
fn bench_parse_orderby_multiple(c: &mut Criterion) {
    c.bench_function("orderby_parse_multiple", |b| {
        b.iter(|| {
            let result = Parser::parse(black_box(ORDERBY_MULTIPLE));
            black_box(result)
        });
    });
}

/// Benchmark: Sort results by similarity score
/// Target: < 200µs for 1000 results
fn bench_sort_by_similarity_score(c: &mut Criterion) {
    let mut group = c.benchmark_group("orderby_sort");

    for size in &[100usize, 500, 1000, 5000] {
        // Generate mock search results with scores
        let results: Vec<(u64, f32)> = (0..*size).map(|i| (i as u64, rand_score(i))).collect();

        group.bench_with_input(BenchmarkId::new("sort_desc", size), size, |b, _| {
            b.iter(|| {
                let mut data = results.clone();
                // Sort by score descending (highest first)
                data.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
                black_box(data)
            });
        });

        group.bench_with_input(BenchmarkId::new("sort_asc", size), size, |b, _| {
            b.iter(|| {
                let mut data = results.clone();
                // Sort by score ascending (lowest first)
                data.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
                black_box(data)
            });
        });
    }

    group.finish();
}

/// Generate deterministic pseudo-random score for benchmarking
#[inline]
fn rand_score(seed: usize) -> f32 {
    let seed = seed as u64;
    // Simple LCG for deterministic "random" scores
    let x = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
    #[allow(clippy::cast_precision_loss)]
    let score = (x % 1000) as f32 / 1000.0;
    score
}

criterion_group!(
    benches,
    bench_parse_orderby_similarity_desc,
    bench_parse_orderby_similarity_asc,
    bench_parse_where_orderby_similarity,
    bench_parse_orderby_multiple,
    bench_sort_by_similarity_score,
);

criterion_main!(benches);
