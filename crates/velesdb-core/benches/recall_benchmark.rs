//! Recall@k benchmark for `VelesDB` search quality validation.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::match_same_arms,
    clippy::unreadable_literal
)]
//!
//! Run with: `cargo bench --bench recall_benchmark`
//!
//! This benchmark measures the **quality** of search results, not just speed.
//! For exact brute-force search, recall should be 100%.
//! For HNSW approximate search, recall depends on the quality profile.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use velesdb_core::distance::DistanceMetric;
use velesdb_core::metrics::{mrr, precision_at_k, recall_at_k};
use velesdb_core::simd;
use velesdb_core::{HnswIndex, HnswParams, SearchQuality, VectorIndex};

/// Generate a random vector using deterministic pseudo-random values
fn generate_vector(dim: usize, seed: u64) -> Vec<f32> {
    let mut state = seed;
    (0..dim)
        .map(|_| {
            // Simple LCG for reproducibility
            state = state.wrapping_mul(1103515245).wrapping_add(12345);
            #[allow(clippy::cast_precision_loss)]
            let val = ((state >> 16) & 0x7FFF) as f32 / 32768.0;
            val * 2.0 - 1.0 // Range [-1, 1]
        })
        .collect()
}

/// Compute exact k-nearest neighbors using brute force
fn brute_force_knn(
    query: &[f32],
    dataset: &[Vec<f32>],
    k: usize,
    metric: DistanceMetric,
) -> Vec<u64> {
    let mut distances: Vec<(u64, f32)> = dataset
        .iter()
        .enumerate()
        .map(|(idx, vec)| {
            let dist = match metric {
                DistanceMetric::Euclidean => simd::euclidean_distance_fast(query, vec),
                DistanceMetric::DotProduct => simd::dot_product_fast(query, vec),
                DistanceMetric::Cosine | DistanceMetric::Hamming | DistanceMetric::Jaccard => {
                    simd::cosine_similarity_fast(query, vec)
                }
            };
            #[allow(clippy::cast_possible_truncation)]
            (idx as u64, dist)
        })
        .collect();

    // Sort by similarity (higher is better for cosine/dot, lower for euclidean)
    match metric {
        DistanceMetric::Euclidean => {
            distances.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        }
        _ => {
            distances.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        }
    }

    distances.into_iter().take(k).map(|(id, _)| id).collect()
}

// Note: recall_at_k, precision_at_k, and mrr are now imported from velesdb_core::metrics

/// Benchmark recall for HNSW index with different quality profiles
fn bench_hnsw_recall(c: &mut Criterion) {
    let mut group = c.benchmark_group("recall_hnsw");
    group.sample_size(20); // Lower sample size for quality benchmarks

    let dim = 128;
    let dataset_sizes = [1000, 10000];
    let k_values = [10, 100];

    for &n in &dataset_sizes {
        // Generate dataset
        let dataset: Vec<Vec<f32>> = (0..n).map(|i| generate_vector(dim, i as u64)).collect();

        // Build HNSW index with max_recall params for high quality
        // M=32, ef_construction=500 for dim<=256
        let params = HnswParams::max_recall(dim);
        let index = HnswIndex::with_params(dim, DistanceMetric::Cosine, params);

        for (idx, vec) in dataset.iter().enumerate() {
            #[allow(clippy::cast_possible_truncation)]
            index.insert(idx as u64, vec);
        }

        for &k in &k_values {
            if k > n {
                continue;
            }

            // Generate query vectors
            let queries: Vec<Vec<f32>> = (0..10)
                .map(|i| generate_vector(dim, (n + i) as u64))
                .collect();

            // Compute ground truth for all queries
            let ground_truths: Vec<Vec<u64>> = queries
                .iter()
                .map(|q| brute_force_knn(q, &dataset, k, DistanceMetric::Cosine))
                .collect();

            // Benchmark different quality profiles
            for quality in [
                SearchQuality::Fast,
                SearchQuality::Balanced,
                SearchQuality::Accurate,
                SearchQuality::HighRecall,
            ] {
                let quality_name = match quality {
                    SearchQuality::Fast => "fast",
                    SearchQuality::Balanced => "balanced",
                    SearchQuality::Accurate => "accurate",
                    SearchQuality::HighRecall => "highrecall",
                    SearchQuality::Custom(_) => "custom",
                };

                group.bench_function(
                    BenchmarkId::new(format!("n{n}_k{k}_{quality_name}"), format!("{n}x{dim}")),
                    |b| {
                        b.iter(|| {
                            let mut total_recall = 0.0;
                            let mut total_mrr = 0.0;

                            for (query, ground_truth) in queries.iter().zip(&ground_truths) {
                                let results = index.search_with_quality(query, k, quality);
                                let result_ids: Vec<u64> =
                                    results.iter().map(|(id, _)| *id).collect();

                                total_recall += recall_at_k(ground_truth, &result_ids);
                                total_mrr += mrr(ground_truth, &result_ids);
                            }

                            // Also compute precision for completeness
                            let last_query = &queries[0];
                            let last_results = index.search_with_quality(last_query, k, quality);
                            let last_ids: Vec<u64> =
                                last_results.iter().map(|(id, _)| *id).collect();
                            let _precision = precision_at_k(&ground_truths[0], &last_ids);

                            #[allow(clippy::cast_precision_loss)]
                            let avg_recall = total_recall / queries.len() as f64;
                            #[allow(clippy::cast_precision_loss)]
                            let avg_mrr = total_mrr / queries.len() as f64;

                            black_box((avg_recall, avg_mrr))
                        });
                    },
                );
            }
        }
    }

    group.finish();
}

/// Compute and print recall statistics (benchmark + one-time stats output)
fn print_recall_stats(c: &mut Criterion) {
    let mut group = c.benchmark_group("recall_stats");
    group.sample_size(10);

    let dim = 128;
    let n = 10000;
    let k = 10;

    // Generate dataset
    let dataset: Vec<Vec<f32>> = (0..n).map(|i| generate_vector(dim, i as u64)).collect();

    // Build HNSW index with max_recall params for high quality
    let params = HnswParams::max_recall(dim);
    let index = HnswIndex::with_params(dim, DistanceMetric::Cosine, params);

    for (idx, vec) in dataset.iter().enumerate() {
        #[allow(clippy::cast_possible_truncation)]
        index.insert(idx as u64, vec);
    }

    // Generate queries
    let queries: Vec<Vec<f32>> = (0..100)
        .map(|i| generate_vector(dim, (n + i) as u64))
        .collect();

    // Compute ground truth
    let ground_truths: Vec<Vec<u64>> = queries
        .iter()
        .map(|q| brute_force_knn(q, &dataset, k, DistanceMetric::Cosine))
        .collect();

    // Compute stats once before benchmark for display
    let mut final_recalls = Vec::new();
    for quality in [
        SearchQuality::Fast,
        SearchQuality::Balanced,
        SearchQuality::Accurate,
        SearchQuality::HighRecall,
    ] {
        let mut total_recall = 0.0;
        for (query, ground_truth) in queries.iter().zip(&ground_truths) {
            let results = index.search_with_quality(query, k, quality);
            let result_ids: Vec<u64> = results.iter().map(|(id, _)| *id).collect();
            total_recall += recall_at_k(ground_truth, &result_ids);
        }
        #[allow(clippy::cast_precision_loss)]
        let avg_recall = total_recall / queries.len() as f64;
        final_recalls.push(avg_recall);
    }

    // Print stats once (before benchmark)
    println!("\n=== Recall@{k} Statistics (n={n}, dim={dim}, M=32, ef_c=500) ===");
    println!("Fast (ef=64):       {:.1}%", final_recalls[0] * 100.0);
    println!("Balanced (ef=128):  {:.1}%", final_recalls[1] * 100.0);
    println!("Accurate (ef=256):  {:.1}%", final_recalls[2] * 100.0);
    println!("HighRecall (ef=512): {:.1}%", final_recalls[3] * 100.0);

    // Benchmark the computation (no print inside)
    group.bench_function("compute_recall_stats", |b| {
        b.iter(|| {
            let mut recalls = Vec::with_capacity(4);

            for quality in [
                SearchQuality::Fast,
                SearchQuality::Balanced,
                SearchQuality::Accurate,
                SearchQuality::HighRecall,
            ] {
                let mut total_recall = 0.0;

                for (query, ground_truth) in queries.iter().zip(&ground_truths) {
                    let results = index.search_with_quality(query, k, quality);
                    let result_ids: Vec<u64> = results.iter().map(|(id, _)| *id).collect();
                    total_recall += recall_at_k(ground_truth, &result_ids);
                }

                #[allow(clippy::cast_precision_loss)]
                let avg_recall = total_recall / queries.len() as f64;
                recalls.push(avg_recall);
            }

            black_box(recalls)
        });
    });

    group.finish();
}

criterion_group!(benches, bench_hnsw_recall, print_recall_stats);
criterion_main!(benches);
