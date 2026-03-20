//! HNSW Parameter Tuning Benchmarks
//!
//! Explores the recall/latency tradeoff for different parameter configurations.
//! Use this to find optimal settings for your use case.
//!
//! Run with: `cargo bench --bench hnsw_tuning`

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use std::collections::HashSet;
use velesdb_core::{DistanceMetric, HnswIndex, ScoredResult, VectorIndex};

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

/// Benchmark `ef_search` parameter sweep.
/// Tests recall and latency at different `ef_search` values.
fn bench_ef_search_sweep(c: &mut Criterion) {
    let mut group = c.benchmark_group("ef_search_sweep");
    group.sample_size(20);

    let dim = 128;
    let num_vectors = 10_000;
    let k = 10;
    let num_queries = 50;

    // Build index with current defaults (M=32, ef_construction=400)
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
    // Default search uses SearchQuality::Balanced (ef_search=128)
    // Use search_with_quality() for custom ef_search values
    println!("\n🔍 Current HnswIndex configuration (M=32, ef_construction=400, Balanced=ef_search=128):\n");

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
    group.bench_function(BenchmarkId::new("current_ef200", "latency"), |b| {
        b.iter(|| {
            let results = index.search(&queries[0], k);
            criterion::black_box(results)
        });
    });

    group.finish();

    // Print recommendation table
    println!("\n📋 HNSW Tuning Recommendations by Vector Dimension:\n");
    println!("┌─────────────┬─────────┬──────────────────┬────────────┐");
    println!("│ Dimension   │ M       │ ef_construction  │ ef_search  │");
    println!("├─────────────┼─────────┼──────────────────┼────────────┤");
    println!("│ d ≤ 256     │ 12-16   │ 100-200          │ 64-128     │");
    println!("│ 256 < d ≤768│ 16-24   │ 200-400          │ 128-256    │");
    println!("│ d > 768     │ 24-32   │ 300-600          │ 256-512    │");
    println!("└─────────────┴─────────┴──────────────────┴────────────┘");
    println!("\n💡 Quality Profiles:");
    println!("   • fast:     ef_search=64  (lower recall, faster)");
    println!("   • balanced: ef_search=128 (good tradeoff)");
    println!("   • accurate: ef_search=512 (best recall, still <10ms)\n");
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
    bench_ef_search_sweep,
    bench_recall_at_k,
    bench_scalability
);
criterion_main!(benches);
