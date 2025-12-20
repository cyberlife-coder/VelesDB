//! WIS-1 Validation Benchmarks
//!
//! Validates the acceptance criteria for WIS-1 (HNSW Index):
//! - Performance < 10ms for 100k vectors search
//! - Recall > 95% on standard benchmarks
//!
//! Run with: `cargo bench --bench wis1_validation`
//!
//! ## Optimizations
//!
//! - Uses `select_nth_unstable_by` for O(n) k-NN instead of O(n log n) sort
//! - Uses `total_cmp` for NaN-safe float comparisons
//! - Tests multiple dimensions (128, 256, 512, 768)

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use std::collections::HashSet;
use velesdb_core::{DistanceMetric, HnswIndex, VectorIndex};

/// Simple LCG random number generator for reproducible benchmarks.
struct SimpleRng {
    state: u64,
}

impl SimpleRng {
    fn new(seed: u64) -> Self {
        Self {
            state: seed.wrapping_add(1),
        }
    }

    #[allow(clippy::cast_precision_loss)]
    fn next_f32(&mut self) -> f32 {
        // LCG parameters from Numerical Recipes
        self.state = self
            .state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        // Convert to [0, 1) range
        (self.state >> 33) as f32 / (1u64 << 31) as f32
    }
}

/// Generates a normalized random vector for benchmarking.
fn generate_vector(dim: usize, seed: u64) -> Vec<f32> {
    let mut rng = SimpleRng::new(seed);
    let mut vec: Vec<f32> = (0..dim).map(|_| rng.next_f32() * 2.0 - 1.0).collect();

    // Normalize for cosine similarity
    let norm: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in &mut vec {
            *x /= norm;
        }
    }
    vec
}

/// Brute-force exact k-NN search for recall calculation.
///
/// Optimized with `select_nth_unstable_by` for O(n) complexity instead of O(n log n) sort.
/// Uses `total_cmp` for NaN-safe float comparisons.
fn brute_force_knn(
    vectors: &[(u64, Vec<f32>)],
    query: &[f32],
    k: usize,
    metric: DistanceMetric,
) -> Vec<u64> {
    let mut distances: Vec<(u64, f32)> = vectors
        .iter()
        .map(|(id, vec)| {
            let dist = match metric {
                DistanceMetric::Cosine => {
                    let dot: f32 = query.iter().zip(vec.iter()).map(|(a, b)| a * b).sum();
                    1.0 - dot // cosine distance
                }
                DistanceMetric::Euclidean => query
                    .iter()
                    .zip(vec.iter())
                    .map(|(a, b)| (a - b).powi(2))
                    .sum::<f32>()
                    .sqrt(),
                DistanceMetric::DotProduct => -query
                    .iter()
                    .zip(vec.iter())
                    .map(|(a, b)| a * b)
                    .sum::<f32>(),
                #[allow(clippy::cast_precision_loss)]
                DistanceMetric::Hamming => query
                    .iter()
                    .zip(vec.iter())
                    .filter(|(a, b)| (**a > 0.5) != (**b > 0.5))
                    .count() as f32,
                #[allow(clippy::cast_precision_loss)]
                DistanceMetric::Jaccard => {
                    let intersection = query
                        .iter()
                        .zip(vec.iter())
                        .filter(|(a, b)| **a > 0.5 && **b > 0.5)
                        .count();
                    let union = query
                        .iter()
                        .zip(vec.iter())
                        .filter(|(a, b)| **a > 0.5 || **b > 0.5)
                        .count();
                    if union == 0 {
                        0.0 // For sorting: lower = more similar
                    } else {
                        1.0 - (intersection as f32 / union as f32)
                    }
                }
            };
            (*id, dist)
        })
        .collect();

    // O(n) selection instead of O(n log n) sort - much faster for large datasets
    // Uses total_cmp for NaN-safe comparison (NaN sorts to end)
    if distances.len() > k {
        distances.select_nth_unstable_by(k, |a, b| a.1.total_cmp(&b.1));
        distances.truncate(k);
        // Sort only the k elements we need
        distances.sort_by(|a, b| a.1.total_cmp(&b.1));
    } else {
        distances.sort_by(|a, b| a.1.total_cmp(&b.1));
    }

    distances.into_iter().map(|(id, _)| id).collect()
}

/// Calculate recall: proportion of true nearest neighbors found.
fn calculate_recall(hnsw_results: &[(u64, f32)], ground_truth: &[u64]) -> f64 {
    let hnsw_ids: HashSet<u64> = hnsw_results.iter().map(|(id, _)| *id).collect();
    let truth_ids: HashSet<u64> = ground_truth.iter().copied().collect();

    let intersection = hnsw_ids.intersection(&truth_ids).count();
    #[allow(clippy::cast_precision_loss)]
    {
        intersection as f64 / ground_truth.len() as f64
    }
}

/// WIS-1 Criterion 1: Performance < 10ms for 100k vectors search
fn bench_100k_search_latency(c: &mut Criterion) {
    let mut group = c.benchmark_group("wis1_100k_search");
    group.sample_size(20); // Reduced for stability

    let dim = 128;
    let num_vectors = 100_000;
    let num_queries = 256; // Pool of queries for stable measurements

    println!(
        "\n=== bench_100k_search_latency ===\n📊 Building index with {num_vectors} vectors (dim={dim})..."
    );

    let index = HnswIndex::new(dim, DistanceMetric::Cosine);

    // Insert 100k vectors (unique IDs only)
    for i in 0..num_vectors {
        let vector = generate_vector(dim, i);
        index.insert(i, &vector);
    }

    // Set searching mode after bulk insertion (required by hnsw_rs)
    index.set_searching_mode();

    println!("✅ Index built with {} vectors", index.len());

    // Pre-generate query pool for stable measurements
    let queries: Vec<Vec<f32>> = (0..num_queries)
        .map(|i| generate_vector(dim, num_vectors + i))
        .collect();

    for k in &[10, 50] {
        let mut query_idx = 0usize;
        group.bench_with_input(
            BenchmarkId::new("search_100k", format!("top_{k}")),
            k,
            |b, &k| {
                b.iter(|| {
                    let query = &queries[query_idx % queries.len()];
                    query_idx = query_idx.wrapping_add(1);
                    let results = index.search(query, k);
                    black_box(results)
                });
            },
        );
    }

    group.finish();
}

/// WIS-1 Criterion 2: Recall > 95%
/// Measures recall by comparing HNSW results to brute-force ground truth.
fn bench_recall_measurement(c: &mut Criterion) {
    let mut group = c.benchmark_group("wis1_recall");
    group.sample_size(10);

    let dim = 128;
    let num_vectors = 10_000; // Smaller for recall calculation (brute force is O(n))
    let k = 10;
    let num_queries = 100;

    println!(
        "\n=== bench_recall_measurement ===\n📊 Measuring recall with {num_vectors} vectors..."
    );

    // Build index
    let index = HnswIndex::new(dim, DistanceMetric::Cosine);
    let mut vectors: Vec<(u64, Vec<f32>)> = Vec::with_capacity(num_vectors);

    #[allow(clippy::cast_sign_loss)]
    for i in 0..num_vectors {
        let id = i as u64;
        let vector = generate_vector(dim, id);
        index.insert(id, &vector);
        vectors.push((id, vector));
    }

    // Set searching mode after bulk insertion
    index.set_searching_mode();

    // Generate queries and measure recall
    let mut total_recall = 0.0;

    #[allow(clippy::cast_sign_loss)]
    for q in 0..num_queries {
        let query = generate_vector(dim, (num_vectors + q) as u64);

        // HNSW search
        let hnsw_results = index.search(&query, k);

        // Brute force ground truth
        let ground_truth = brute_force_knn(&vectors, &query, k, DistanceMetric::Cosine);

        // Calculate recall (comparing IDs only, not scores)
        let recall = calculate_recall(&hnsw_results, &ground_truth);
        total_recall += recall;
    }

    #[allow(clippy::cast_precision_loss)]
    let avg_recall = total_recall / num_queries as f64;
    println!("\n🎯 Average Recall@{k}: {:.2}%", avg_recall * 100.0);
    println!(
        "   {} WIS-1 Criterion: Recall > 95%\n",
        if avg_recall >= 0.95 { "✅" } else { "❌" }
    );

    // Benchmark search latency on 10k index (correctly named)
    group.bench_function("search_10k_top10", |b| {
        let query = generate_vector(dim, 999_999);
        b.iter(|| {
            let results = index.search(&query, k);
            black_box(results)
        });
    });

    group.finish();
}

/// Combined validation for Cosine and Euclidean metrics.
/// Note: `DotProduct` excluded due to `hnsw_rs` constraint (requires non-negative dot products)
fn bench_all_metrics(c: &mut Criterion) {
    let mut group = c.benchmark_group("wis1_all_metrics");
    group.sample_size(20);

    let dim = 128;
    let num_vectors = 50_000;

    println!("\n=== bench_all_metrics ===");

    // DotProduct excluded - hnsw_rs DistDot requires non-negative dot products
    for metric in &[DistanceMetric::Cosine, DistanceMetric::Euclidean] {
        let index = HnswIndex::new(dim, *metric);

        #[allow(clippy::cast_sign_loss)]
        for i in 0..num_vectors {
            let id = i as u64;
            let vector = generate_vector(dim, id);
            index.insert(id, &vector);
        }

        // Set searching mode after bulk insertion
        index.set_searching_mode();

        let query = generate_vector(dim, 999_999);

        group.bench_with_input(
            BenchmarkId::new("search_50k", format!("{metric:?}")),
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

/// Benchmark recall on 100k vectors (validates HNSW params at scale).
/// Note: This test is slower due to brute-force ground truth computation.
fn bench_recall_100k(c: &mut Criterion) {
    let mut group = c.benchmark_group("wis1_recall_100k");
    group.sample_size(10);

    let dim = 128;
    let num_vectors = 100_000;
    let k = 10;
    let num_queries = 20; // Fewer queries due to slow brute-force on 100k

    println!(
        "\n=== bench_recall_100k ===\n📊 Measuring recall on {num_vectors} vectors (may take a while)..."
    );

    let index = HnswIndex::new(dim, DistanceMetric::Cosine);
    let mut vectors: Vec<(u64, Vec<f32>)> = Vec::with_capacity(num_vectors);

    #[allow(clippy::cast_sign_loss)]
    for i in 0..num_vectors {
        let id = i as u64;
        let vector = generate_vector(dim, id);
        index.insert(id, &vector);
        vectors.push((id, vector));
    }

    index.set_searching_mode();
    println!("✅ Index built with {} vectors", index.len());

    // Measure recall
    let mut total_recall = 0.0;
    #[allow(clippy::cast_sign_loss)]
    for q in 0..num_queries {
        let query = generate_vector(dim, (num_vectors + q) as u64);
        let hnsw_results = index.search(&query, k);
        let ground_truth = brute_force_knn(&vectors, &query, k, DistanceMetric::Cosine);
        total_recall += calculate_recall(&hnsw_results, &ground_truth);
    }

    #[allow(clippy::cast_precision_loss)]
    let avg_recall = total_recall / num_queries as f64;
    println!(
        "\n🎯 Recall@{k} on 100k vectors: {:.2}%",
        avg_recall * 100.0
    );
    println!(
        "   {} WIS-1 Criterion: Recall > 95%\n",
        if avg_recall >= 0.95 { "✅" } else { "❌" }
    );

    group.bench_function("search_100k_recall_test", |b| {
        let query = generate_vector(dim, 999_999);
        b.iter(|| black_box(index.search(&query, k)));
    });

    group.finish();
}

/// Benchmark performance across different vector dimensions.
/// Tests if performance degrades significantly at higher dimensions.
fn bench_dimensions(c: &mut Criterion) {
    let mut group = c.benchmark_group("wis1_dimensions");
    group.sample_size(10);

    let num_vectors = 10_000;
    let k = 10;

    println!("\n=== bench_dimensions ===\n📊 Testing performance across dimensions...\n");

    for dim in [128, 256, 512, 768] {
        let index = HnswIndex::new(dim, DistanceMetric::Cosine);

        #[allow(clippy::cast_sign_loss)]
        for i in 0..num_vectors {
            let id = i as u64;
            let vector = generate_vector(dim, id);
            index.insert(id, &vector);
        }

        index.set_searching_mode();

        let query = generate_vector(dim, 999_999);

        group.bench_with_input(
            BenchmarkId::new("search_10k", format!("dim_{dim}")),
            &dim,
            |b, _| b.iter(|| black_box(index.search(&query, k))),
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_recall_measurement,
    bench_100k_search_latency,
    bench_all_metrics,
    bench_recall_100k,
    bench_dimensions
);
criterion_main!(benches);
