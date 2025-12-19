//! Benchmark for VelesQL parser performance.

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use velesdb_core::velesql::Parser;

/// Simple SELECT query
const SIMPLE_QUERY: &str = "SELECT * FROM documents LIMIT 10";

/// Vector search query
const VECTOR_QUERY: &str = "SELECT * FROM documents WHERE vector NEAR $v LIMIT 10";

/// Complex query with filters
const COMPLEX_QUERY: &str = r#"
SELECT id, payload.title, score 
FROM documents 
WHERE vector NEAR COSINE $query_vector
  AND category = 'tech'
  AND price > 100
  AND tags IN ('rust', 'performance', 'database')
LIMIT 20 OFFSET 5
"#;

/// Query with multiple conditions
const MULTI_CONDITION_QUERY: &str = r#"
SELECT * FROM docs 
WHERE category = 'tech' 
  AND price BETWEEN 10 AND 1000 
  AND title LIKE '%rust%'
  AND deleted_at IS NULL
LIMIT 50
"#;

fn bench_parse_simple(c: &mut Criterion) {
    c.bench_function("velesql_parse_simple", |b| {
        b.iter(|| {
            let _ = black_box(Parser::parse(SIMPLE_QUERY));
        });
    });
}

fn bench_parse_vector(c: &mut Criterion) {
    c.bench_function("velesql_parse_vector", |b| {
        b.iter(|| {
            let _ = black_box(Parser::parse(VECTOR_QUERY));
        });
    });
}

fn bench_parse_complex(c: &mut Criterion) {
    c.bench_function("velesql_parse_complex", |b| {
        b.iter(|| {
            let _ = black_box(Parser::parse(COMPLEX_QUERY));
        });
    });
}

fn bench_parse_multi_condition(c: &mut Criterion) {
    c.bench_function("velesql_parse_multi_condition", |b| {
        b.iter(|| {
            let _ = black_box(Parser::parse(MULTI_CONDITION_QUERY));
        });
    });
}

fn bench_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("velesql_throughput");

    // Measure queries per second
    group.throughput(Throughput::Elements(1));

    group.bench_function("simple_qps", |b| {
        b.iter(|| {
            let _ = black_box(Parser::parse(SIMPLE_QUERY));
        });
    });

    group.bench_function("vector_qps", |b| {
        b.iter(|| {
            let _ = black_box(Parser::parse(VECTOR_QUERY));
        });
    });

    group.bench_function("complex_qps", |b| {
        b.iter(|| {
            let _ = black_box(Parser::parse(COMPLEX_QUERY));
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_parse_simple,
    bench_parse_vector,
    bench_parse_complex,
    bench_parse_multi_condition,
    bench_throughput
);

criterion_main!(benches);
