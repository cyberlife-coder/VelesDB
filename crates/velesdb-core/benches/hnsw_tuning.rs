#![warn(clippy::significant_drop_tightening)]
//! HNSW Parameter Tuning Benchmarks
//!
//! Explores the recall/latency tradeoff for different parameter configurations.
//! Use this to find optimal settings for your use case.
//!
//! Run with: `cargo bench --bench hnsw_tuning`

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use std::collections::HashSet;
use velesdb_core::{
    DistanceMetric, HnswIndex, HnswParams, ScoredResult, SearchQuality, VectorIndex,
};

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
        self.state = self
            .state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.state >> 33) as f32 / (1u64 << 31) as f32
    }
}

/// Generates a normalized random vector.
fn generate_vector(dim: usize, seed: u64) -> Vec<f32> {
    let mut rng = SimpleRng::new(seed);
    let mut vec: Vec<f32> = (0..dim).map(|_| rng.next_f32() * 2.0 - 1.0).collect();

    let norm: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in &mut vec {
            *x /= norm;
        }
    }
    vec
}

/// Brute-force exact k-NN search for recall calculation.
fn brute_force_knn(vectors: &[(u64, Vec<f32>)], query: &[f32], k: usize) -> Vec<u64> {
    let mut distances: Vec<(u64, f32)> = vectors
        .iter()
        .map(|(id, vec)| {
            let dot: f32 = query.iter().zip(vec.iter()).map(|(a, b)| a * b).sum();
            (*id, 1.0 - dot) // cosine distance
        })
        .collect();

    distances.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
    distances.into_iter().take(k).map(|(id, _)| id).collect()
}

/// Calculate recall.
fn calculate_recall(hnsw_results: &[ScoredResult], ground_truth: &[u64]) -> f64 {
    let hnsw_ids: HashSet<u64> = hnsw_results.iter().map(|sr| sr.id).collect();
    let truth_ids: HashSet<u64> = ground_truth.iter().copied().collect();
    let intersection = hnsw_ids.intersection(&truth_ids).count();
    #[allow(clippy::cast_precision_loss)]
    {
        intersection as f64 / ground_truth.len() as f64
    }
}

/// Measures recall and latency of the default Balanced search.
#[expect(clippy::significant_drop_tightening)] // Reason: the guard under test is held to the assertion on purpose
fn bench_default_search(c: &mut Criterion) {
    let mut group = c.benchmark_group("default_search");
    group.sample_size(20);

    let dim = 128;
    let num_vectors = 10_000;
    let k = 10;
    let num_queries = 50;

    // Build index with the current defaults, `HnswParams::auto(dim)`
    let index = HnswIndex::new(dim, DistanceMetric::Cosine).unwrap();
    let mut vectors: Vec<(u64, Vec<f32>)> = Vec::with_capacity(num_vectors);

    println!("\n📊 Building index: {num_vectors} vectors, dim={dim}");

    #[allow(clippy::cast_sign_loss)]
    for i in 0..num_vectors {
        let id = i as u64;
        let vector = generate_vector(dim, id);
        index.insert(id, &vector);
        vectors.push((id, vector));
    }

    // Set searching mode after bulk insertion
    index.set_searching_mode();

    // Generate queries and ground truth
    #[allow(clippy::cast_sign_loss)]
    let queries: Vec<Vec<f32>> = (0..num_queries)
        .map(|i| generate_vector(dim, (num_vectors + i) as u64))
        .collect();

    let ground_truths: Vec<Vec<u64>> = queries
        .iter()
        .map(|q| brute_force_knn(&vectors, q, k))
        .collect();

    // Test different ef_search values
    // Default search uses SearchQuality::Balanced
    let defaults = HnswParams::auto(dim);
    println!(
        "\n🔍 Current configuration: M={}, ef_construction={}, Balanced ef_search={}\n",
        defaults.max_connections,
        defaults.ef_construction,
        SearchQuality::Balanced.ef_search_for_scale(k, index.len())
    );

    // Measure recall with current settings
    let mut total_recall = 0.0;
    for (query, truth) in queries.iter().zip(ground_truths.iter()) {
        let results = index.search(query, k);
        total_recall += calculate_recall(&results, truth);
    }
    #[allow(clippy::cast_precision_loss)]
    let avg_recall = total_recall / num_queries as f64;
    println!("   Recall@{k}: {:.2}%", avg_recall * 100.0);

    // Benchmark latency
    group.bench_function(BenchmarkId::new("current_balanced", "latency"), |b| {
        b.iter(|| {
            let results = index.search(&queries[0], k);
            criterion::black_box(results)
        });
    });

    group.finish();

    // Defaults this crate derives, by dimension
    println!("\n📋 `HnswParams::auto` by vector dimension:\n");
    for d in [128, 768, 1536] {
        let p = HnswParams::auto(d);
        println!(
            "   d = {d:>4}: M = {}, ef_construction = {}",
            p.max_connections, p.ef_construction
        );
    }
    println!("\n💡 Quality profiles (base ef_search at k = {k}):");
    println!(
        "   • fast:     ef_search={} (lower recall, faster)",
        SearchQuality::Fast.ef_search(k)
    );
    println!(
        "   • balanced: ef_search={} (good tradeoff)",
        SearchQuality::Balanced.ef_search(k)
    );
    println!(
        "   • accurate: ef_search={} (best recall)\n",
        SearchQuality::Accurate.ef_search(k)
    );
}

/// Test recall at different k values.
fn bench_recall_at_k(c: &mut Criterion) {
    let mut group = c.benchmark_group("recall_at_k");
    group.sample_size(10);

    let dim = 128;
    let num_vectors = 10_000;
    let num_queries = 50;

    let index = HnswIndex::new(dim, DistanceMetric::Cosine).unwrap();
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

    #[allow(clippy::cast_sign_loss)]
    let queries: Vec<Vec<f32>> = (0..num_queries)
        .map(|i| generate_vector(dim, (num_vectors + i) as u64))
        .collect();

    println!("\n📊 Recall at different k values:\n");

    for k in [10, 20, 50, 100] {
        let ground_truths: Vec<Vec<u64>> = queries
            .iter()
            .map(|q| brute_force_knn(&vectors, q, k))
            .collect();

        let mut total_recall = 0.0;
        for (query, truth) in queries.iter().zip(ground_truths.iter()) {
            let results = index.search(query, k);
            total_recall += calculate_recall(&results, truth);
        }
        #[allow(clippy::cast_precision_loss)]
        let avg_recall = total_recall / num_queries as f64;
        println!("   Recall@{k}: {:.2}%", avg_recall * 100.0);

        group.bench_function(BenchmarkId::new("search", format!("top_{k}")), |b| {
            b.iter(|| {
                let results = index.search(&queries[0], k);
                criterion::black_box(results)
            });
        });
    }

    group.finish();
}

/// Benchmark scalability: 10k, 50k, 100k vectors.
fn bench_scalability(c: &mut Criterion) {
    let mut group = c.benchmark_group("scalability");
    group.sample_size(20);

    let dim = 128;
    let k = 10;

    println!("\n📊 Scalability test (latency vs index size):\n");

    for num_vectors in [10_000, 50_000, 100_000] {
        let index = HnswIndex::new(dim, DistanceMetric::Cosine).unwrap();

        #[allow(clippy::cast_sign_loss)]
        for i in 0..num_vectors {
            let id = i as u64;
            let vector = generate_vector(dim, id);
            index.insert(id, &vector);
        }

        // Set searching mode after bulk insertion
        index.set_searching_mode();

        let query = generate_vector(dim, 999_999);

        group.bench_function(
            BenchmarkId::new("search", format!("{}k_vectors", num_vectors / 1000)),
            |b| {
                b.iter(|| {
                    let results = index.search(&query, k);
                    criterion::black_box(results)
                });
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_default_search,
    bench_recall_at_k,
    bench_scalability
);
criterion_main!(benches);
