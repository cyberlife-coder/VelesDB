//! Benchmark for `VelesQL` parser and cache performance.

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use velesdb_core::velesql::{Parser, QueryCache};

/// Simple SELECT query
const SIMPLE_QUERY: &str = "SELECT * FROM documents LIMIT 10";

/// Vector search query
const VECTOR_QUERY: &str = "SELECT * FROM documents WHERE vector NEAR $v LIMIT 10";

/// Complex query with filters
const COMPLEX_QUERY: &str = r"
SELECT id, payload.title, score 
FROM documents 
WHERE vector NEAR COSINE $query_vector
  AND category = 'tech'
  AND price > 100
  AND tags IN ('rust', 'performance', 'database')
LIMIT 20 OFFSET 5
";

/// Query with multiple conditions
const MULTI_CONDITION_QUERY: &str = r"
SELECT * FROM docs 
WHERE category = 'tech' 
  AND price BETWEEN 10 AND 1000 
  AND title LIKE '%rust%'
  AND deleted_at IS NULL
LIMIT 50
";

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

/// Benchmark cache hit performance
fn bench_cache_hit(c: &mut Criterion) {
    let cache = QueryCache::new(1000);
    // Warm up cache
    let _ = cache.parse(SIMPLE_QUERY);

    c.bench_function("velesql_cache_hit", |b| {
        b.iter(|| {
            let _ = black_box(cache.parse(SIMPLE_QUERY));
        });
    });
}

/// Benchmark cache vs direct parsing
fn bench_cache_vs_direct(c: &mut Criterion) {
    let mut group = c.benchmark_group("velesql_cache_comparison");
    group.throughput(Throughput::Elements(1));

    // Direct parsing (no cache)
    group.bench_function("direct_parse", |b| {
        b.iter(|| {
            let _ = black_box(Parser::parse(COMPLEX_QUERY));
        });
    });

    // Cached parsing (cache miss)
    group.bench_function("cache_miss", |b| {
        b.iter_custom(|iters| {
            let start = std::time::Instant::now();
            for i in 0..iters {
                let cache = QueryCache::new(1000);
                let query = format!("SELECT * FROM table_{i} LIMIT 10");
                let _ = black_box(cache.parse(&query));
            }
            start.elapsed()
        });
    });

    // Cached parsing (cache hit)
    let cache = QueryCache::new(1000);
    let _ = cache.parse(COMPLEX_QUERY);
    group.bench_function("cache_hit", |b| {
        b.iter(|| {
            let _ = black_box(cache.parse(COMPLEX_QUERY));
        });
    });

    group.finish();
}

/// Benchmark realistic workload with mixed queries
fn bench_realistic_workload(c: &mut Criterion) {
    let queries = [
        "SELECT * FROM docs LIMIT 10",
        "SELECT * FROM docs WHERE vector NEAR $v LIMIT 5",
        "SELECT id, title FROM docs WHERE category = 'tech'",
        "SELECT * FROM docs LIMIT 10",                     // Repeat
        "SELECT * FROM docs WHERE vector NEAR $v LIMIT 5", // Repeat
    ];

    let mut group = c.benchmark_group("velesql_realistic");

    // Without cache
    group.bench_function("without_cache", |b| {
        b.iter(|| {
            for q in &queries {
                let _ = black_box(Parser::parse(q));
            }
        });
    });

    // With cache
    let cache = QueryCache::new(100);
    group.bench_function("with_cache", |b| {
        b.iter(|| {
            for q in &queries {
                let _ = black_box(cache.parse(q));
            }
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
    bench_throughput,
    bench_cache_hit,
    bench_cache_vs_direct,
    bench_realistic_workload
);

criterion_main!(benches);
