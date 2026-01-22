//! Benchmark for `similarity()` function and fusion strategies (EPIC-008).
//!
//! Measures:
//! - `VelesQL` parser performance for `similarity()` queries
//! - Fusion strategy computation (RRF, Weighted, Average, Maximum)
//! - End-to-end similarity filtering

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use velesdb_core::velesql::Parser;
use velesdb_core::FusionStrategy;

// =============================================================================
// VelesQL Similarity Parser Benchmarks
// =============================================================================

/// Similarity query with threshold
const SIMILARITY_GT: &str = "SELECT * FROM docs WHERE similarity(embedding, $vec) > 0.8 LIMIT 10";

/// Similarity query with >= operator
const SIMILARITY_GTE: &str =
    "SELECT * FROM docs WHERE similarity(embedding, $vec) >= 0.75 LIMIT 20";

/// Similarity query with < operator (rare use case)
const SIMILARITY_LT: &str = "SELECT * FROM docs WHERE similarity(embedding, $vec) < 0.3 LIMIT 5";

/// Complex query combining similarity with other filters
const SIMILARITY_COMPLEX: &str = r"
SELECT id, payload.title, score 
FROM documents 
WHERE similarity(embedding, $query_vec) >= 0.7
  AND category = 'tech'
  AND published = true
LIMIT 50
";

fn bench_parse_similarity_gt(c: &mut Criterion) {
    c.bench_function("similarity_parse_gt", |b| {
        b.iter(|| {
            let _ = black_box(Parser::parse(SIMILARITY_GT));
        });
    });
}

fn bench_parse_similarity_gte(c: &mut Criterion) {
    c.bench_function("similarity_parse_gte", |b| {
        b.iter(|| {
            let _ = black_box(Parser::parse(SIMILARITY_GTE));
        });
    });
}

fn bench_parse_similarity_lt(c: &mut Criterion) {
    c.bench_function("similarity_parse_lt", |b| {
        b.iter(|| {
            let _ = black_box(Parser::parse(SIMILARITY_LT));
        });
    });
}

fn bench_parse_similarity_complex(c: &mut Criterion) {
    c.bench_function("similarity_parse_complex", |b| {
        b.iter(|| {
            let _ = black_box(Parser::parse(SIMILARITY_COMPLEX));
        });
    });
}

// =============================================================================
// Fusion Strategy Benchmarks
// =============================================================================

/// Generate test data for fusion benchmarks
fn generate_fusion_data(size: usize) -> Vec<Vec<(u64, f32)>> {
    let mut rng_seed = 42u64;
    let next_rand = |seed: &mut u64| -> f32 {
        *seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
        #[allow(clippy::cast_precision_loss)]
        let score = ((*seed >> 16) & 0x7fff) as f32 / 32768.0;
        score
    };

    (0..3)
        .map(|_| {
            (0..size)
                .map(|i| {
                    let score = next_rand(&mut rng_seed);
                    (i as u64, score)
                })
                .collect()
        })
        .collect()
}

fn bench_fusion_rrf(c: &mut Criterion) {
    let mut group = c.benchmark_group("fusion_rrf");

    for size in [100, 1000, 10000] {
        let data = generate_fusion_data(size);

        group.throughput(Throughput::Elements(size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), &data, |b, input| {
            let strategy = FusionStrategy::RRF { k: 60 };
            b.iter(|| {
                let _ = black_box(strategy.fuse(input.clone()));
            });
        });
    }

    group.finish();
}

fn bench_fusion_weighted(c: &mut Criterion) {
    let mut group = c.benchmark_group("fusion_weighted");

    for size in [100, 1000, 10000] {
        let data = generate_fusion_data(size);

        group.throughput(Throughput::Elements(size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), &data, |b, input| {
            let strategy = FusionStrategy::Weighted {
                avg_weight: 0.5,
                max_weight: 0.3,
                hit_weight: 0.2,
            };
            b.iter(|| {
                let _ = black_box(strategy.fuse(input.clone()));
            });
        });
    }

    group.finish();
}

fn bench_fusion_average(c: &mut Criterion) {
    let mut group = c.benchmark_group("fusion_average");

    for size in [100, 1000, 10000] {
        let data = generate_fusion_data(size);

        group.throughput(Throughput::Elements(size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), &data, |b, input| {
            let strategy = FusionStrategy::Average;
            b.iter(|| {
                let _ = black_box(strategy.fuse(input.clone()));
            });
        });
    }

    group.finish();
}

fn bench_fusion_maximum(c: &mut Criterion) {
    let mut group = c.benchmark_group("fusion_maximum");

    for size in [100, 1000, 10000] {
        let data = generate_fusion_data(size);

        group.throughput(Throughput::Elements(size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), &data, |b, input| {
            let strategy = FusionStrategy::Maximum;
            b.iter(|| {
                let _ = black_box(strategy.fuse(input.clone()));
            });
        });
    }

    group.finish();
}

// =============================================================================
// Comparison: All Fusion Strategies
// =============================================================================

fn bench_fusion_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("fusion_comparison_1000");
    let size = 1000;
    let data = generate_fusion_data(size);

    group.throughput(Throughput::Elements(size as u64));

    group.bench_function("average", |b| {
        let strategy = FusionStrategy::Average;
        b.iter(|| black_box(strategy.fuse(data.clone())));
    });

    group.bench_function("maximum", |b| {
        let strategy = FusionStrategy::Maximum;
        b.iter(|| black_box(strategy.fuse(data.clone())));
    });

    group.bench_function("rrf_k60", |b| {
        let strategy = FusionStrategy::RRF { k: 60 };
        b.iter(|| black_box(strategy.fuse(data.clone())));
    });

    group.bench_function("weighted", |b| {
        let strategy = FusionStrategy::Weighted {
            avg_weight: 0.5,
            max_weight: 0.3,
            hit_weight: 0.2,
        };
        b.iter(|| black_box(strategy.fuse(data.clone())));
    });

    group.finish();
}

// =============================================================================
// Parser Throughput
// =============================================================================

fn bench_similarity_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("similarity_throughput");
    group.throughput(Throughput::Elements(1));

    group.bench_function("simple_similarity", |b| {
        b.iter(|| black_box(Parser::parse(SIMILARITY_GT)));
    });

    group.bench_function("complex_similarity", |b| {
        b.iter(|| black_box(Parser::parse(SIMILARITY_COMPLEX)));
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_parse_similarity_gt,
    bench_parse_similarity_gte,
    bench_parse_similarity_lt,
    bench_parse_similarity_complex,
    bench_fusion_rrf,
    bench_fusion_weighted,
    bench_fusion_average,
    bench_fusion_maximum,
    bench_fusion_comparison,
    bench_similarity_throughput,
);

criterion_main!(benches);
