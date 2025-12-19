//! HNSW Index Performance Benchmarks
//!
//! Run with: `cargo bench --bench hnsw_benchmark`

#![allow(clippy::cast_precision_loss)]

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use velesdb_core::{Collection, DistanceMetric, HnswIndex, Point, VectorIndex};

/// Generates a random-ish vector for benchmarking.
fn generate_vector(dim: usize, seed: u64) -> Vec<f32> {
    (0..dim)
        .map(|i| ((seed as f32 * 0.1 + i as f32 * 0.01).sin() + 1.0) / 2.0)
        .collect()
}

/// Benchmark HNSW index insertion performance (sequential).
fn bench_hnsw_insert(c: &mut Criterion) {
    let mut group = c.benchmark_group("hnsw_insert");

    for &count in &[1000_u64, 10_000_u64] {
        let dim = 768;
        group.throughput(Throughput::Elements(count));

        group.bench_with_input(
            BenchmarkId::new("sequential", format!("{count}x{dim}d")),
            &count,
            |b, &count| {
                b.iter(|| {
                    let index = HnswIndex::new(dim, DistanceMetric::Cosine);
                    for i in 0..count {
                        let vector = generate_vector(dim, i);
                        index.insert(i, &vector);
                    }
                    black_box(index.len())
                });
            },
        );
    }

    group.finish();
}

/// Benchmark HNSW index parallel insertion performance.
fn bench_hnsw_insert_parallel(c: &mut Criterion) {
    let mut group = c.benchmark_group("hnsw_insert_parallel");

    for &count in &[1000_u64, 10_000_u64] {
        let dim = 768;
        group.throughput(Throughput::Elements(count));

        group.bench_with_input(
            BenchmarkId::new("parallel", format!("{count}x{dim}d")),
            &count,
            |b, &count| {
                // Pre-generate vectors outside the benchmark loop
                let vectors: Vec<(u64, Vec<f32>)> =
                    (0..count).map(|i| (i, generate_vector(dim, i))).collect();

                b.iter(|| {
                    let index = HnswIndex::new(dim, DistanceMetric::Cosine);
                    let inserted = index.insert_batch_parallel(vectors.clone());
                    index.set_searching_mode();
                    black_box(inserted)
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

    for &k in &[10_usize, 50, 100] {
        group.bench_with_input(BenchmarkId::new("top_k", k), &k, |b, &k| {
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
///
/// Note: `DotProduct` excluded as `hnsw_rs` `DistDot` requires non-negative dot products.
fn bench_distance_metrics(c: &mut Criterion) {
    let mut group = c.benchmark_group("distance_metrics");

    let dim = 768;
    let query = generate_vector(dim, 0);

    // Only Cosine and Euclidean - DotProduct requires special vector constraints
    for &metric in &[DistanceMetric::Cosine, DistanceMetric::Euclidean] {
        let index = HnswIndex::new(dim, metric);

        // Populate
        for i in 0_u64..5000 {
            let vector = generate_vector(dim, i);
            index.insert(i, &vector);
        }

        group.bench_with_input(
            BenchmarkId::new("search", format!("{metric:?}")),
            &metric,
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

/// Validate recall ≥95% at different dimensions (WIS-12 acceptance criteria).
///
/// Recall = |HNSW results ∩ Brute-force results| / k
fn bench_recall_validation(c: &mut Criterion) {
    let mut group = c.benchmark_group("recall_validation");
    group.sample_size(10); // Fewer samples since we're measuring recall, not time

    for &dim in &[128_usize, 384, 768, 1536] {
        let n_vectors = 5000_u64;
        let k = 10_usize;

        // Build index with auto-tuned params
        let index = HnswIndex::new(dim, DistanceMetric::Cosine);
        let vectors: Vec<Vec<f32>> = (0..n_vectors).map(|i| generate_vector(dim, i)).collect();

        for (i, v) in vectors.iter().enumerate() {
            index.insert(i as u64, v);
        }

        // Generate test queries
        let n_queries = 100_usize;
        let queries: Vec<Vec<f32>> = (0..n_queries)
            .map(|i| generate_vector(dim, n_vectors + i as u64))
            .collect();

        group.bench_with_input(
            BenchmarkId::new("recall", format!("{dim}d")),
            &dim,
            |b, _| {
                b.iter(|| {
                    let mut total_recall = 0.0_f64;

                    for query in &queries {
                        // HNSW search
                        let hnsw_results: Vec<u64> =
                            index.search(query, k).iter().map(|(id, _)| *id).collect();

                        // Brute-force ground truth
                        let mut distances: Vec<(u64, f32)> = vectors
                            .iter()
                            .enumerate()
                            .map(|(i, v)| {
                                let dist =
                                    1.0 - velesdb_core::simd::cosine_similarity_fast(query, v);
                                (i as u64, dist)
                            })
                            .collect();
                        distances.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
                        let ground_truth: Vec<u64> =
                            distances.iter().take(k).map(|(id, _)| *id).collect();

                        // Calculate recall
                        let hits = hnsw_results
                            .iter()
                            .filter(|id| ground_truth.contains(id))
                            .count();
                        total_recall += hits as f64 / k as f64;
                    }

                    let avg_recall = total_recall / n_queries as f64;
                    // Assert recall ≥ 95%
                    assert!(
                        avg_recall >= 0.95,
                        "Recall {avg_recall:.2}% < 95% for dim={dim}"
                    );
                    black_box(avg_recall)
                });
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_hnsw_insert,
    bench_hnsw_insert_parallel,
    bench_hnsw_search_latency,
    bench_hnsw_search_throughput,
    bench_collection_search,
    bench_distance_metrics,
    bench_recall_validation
);
criterion_main!(benches);
