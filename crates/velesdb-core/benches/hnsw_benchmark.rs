//! HNSW Index Performance Benchmarks
//!
//! Run with: `cargo bench --bench hnsw_benchmark`

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use velesdb_core::{Collection, DistanceMetric, HnswIndex, Point, VectorIndex};

/// Generates a random-ish vector for benchmarking.
fn generate_vector(dim: usize, seed: u64) -> Vec<f32> {
    (0..dim)
        .map(|i| ((seed as f32 * 0.1 + i as f32 * 0.01).sin() + 1.0) / 2.0)
        .collect()
}

/// Benchmark HNSW index insertion performance.
fn bench_hnsw_insert(c: &mut Criterion) {
    let mut group = c.benchmark_group("hnsw_insert");

    for count in [1000, 10_000].iter() {
        let dim = 768;
        group.throughput(Throughput::Elements(*count as u64));

        group.bench_with_input(
            BenchmarkId::new("vectors", format!("{}x{}d", count, dim)),
            count,
            |b, &count| {
                b.iter(|| {
                    let index = HnswIndex::new(dim, DistanceMetric::Cosine);
                    for i in 0..count {
                        let vector = generate_vector(dim, i as u64);
                        index.insert(i as u64, &vector);
                    }
                    black_box(index.len())
                });
            },
        );
    }

    group.finish();
}

/// Benchmark HNSW index search latency.
fn bench_hnsw_search_latency(c: &mut Criterion) {
    let mut group = c.benchmark_group("hnsw_search_latency");

    // Pre-populate index
    let dim = 768;
    let index = HnswIndex::new(dim, DistanceMetric::Cosine);

    for i in 0..10_000 {
        let vector = generate_vector(dim, i);
        index.insert(i, &vector);
    }

    let query = generate_vector(dim, 99999);

    for k in [10, 50, 100].iter() {
        group.bench_with_input(BenchmarkId::new("top_k", k), k, |b, &k| {
            b.iter(|| {
                let results = index.search(&query, k);
                black_box(results)
            });
        });
    }

    group.finish();
}

/// Benchmark HNSW search throughput (queries per second).
fn bench_hnsw_search_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("hnsw_search_throughput");

    let dim = 768;
    let index = HnswIndex::new(dim, DistanceMetric::Cosine);

    // Populate with 10k vectors
    for i in 0..10_000 {
        let vector = generate_vector(dim, i);
        index.insert(i, &vector);
    }

    // Pre-generate queries
    let queries: Vec<Vec<f32>> = (0..100)
        .map(|i| generate_vector(dim, 100_000 + i))
        .collect();

    group.throughput(Throughput::Elements(queries.len() as u64));
    group.bench_function("100_queries_top10", |b| {
        b.iter(|| {
            for query in &queries {
                let results = index.search(query, 10);
                black_box(results);
            }
        });
    });

    group.finish();
}

/// Benchmark Collection with HNSW vs theoretical brute force.
fn bench_collection_search(c: &mut Criterion) {
    let mut group = c.benchmark_group("collection_search");

    let dim = 768;
    let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let path = temp_dir.path().join("bench_collection");

    let collection =
        Collection::create(path, dim, DistanceMetric::Cosine).expect("Failed to create collection");

    // Insert 10k points
    let points: Vec<Point> = (0..10_000u64)
        .map(|i| Point::without_payload(i, generate_vector(dim, i)))
        .collect();

    collection.upsert(points).expect("Failed to upsert");

    let query = generate_vector(dim, 99999);

    group.bench_function("search_10k_top10", |b| {
        b.iter(|| {
            let results = collection.search(&query, 10);
            black_box(results)
        });
    });

    group.finish();
}

/// Compare different distance metrics.
fn bench_distance_metrics(c: &mut Criterion) {
    let mut group = c.benchmark_group("distance_metrics");

    let dim = 768;
    let query = generate_vector(dim, 0);

    for metric in [
        DistanceMetric::Cosine,
        DistanceMetric::Euclidean,
        DistanceMetric::DotProduct,
    ]
    .iter()
    {
        let index = HnswIndex::new(dim, *metric);

        // Populate
        for i in 0..5000 {
            let vector = generate_vector(dim, i);
            index.insert(i, &vector);
        }

        group.bench_with_input(
            BenchmarkId::new("search", format!("{:?}", metric)),
            metric,
            |b, _| {
                b.iter(|| {
                    let results = index.search(&query, 10);
                    black_box(results)
                });
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_hnsw_insert,
    bench_hnsw_search_latency,
    bench_hnsw_search_throughput,
    bench_collection_search,
    bench_distance_metrics
);
criterion_main!(benches);
