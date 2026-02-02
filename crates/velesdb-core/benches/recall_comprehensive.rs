//! Comprehensive recall & latency benchmark for documentation.
//!
//! Run with: `cargo bench --bench recall_comprehensive --release`
//!
//! Measures recall@10 and search latency (P50) for:
//! - 10K vectors / 128D
//! - 100K vectors / 768D

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use std::time::Instant;
use velesdb_core::distance::DistanceMetric;
use velesdb_core::metrics::recall_at_k;
use velesdb_core::simd_native;
use velesdb_core::{HnswIndex, HnswParams, SearchQuality, VectorIndex};

fn generate_vector(dim: usize, seed: u64) -> Vec<f32> {
    let mut state = seed;
    (0..dim)
        .map(|_| {
            state = state.wrapping_mul(1_103_515_245).wrapping_add(12345);
            #[allow(clippy::cast_precision_loss)]
            let val = ((state >> 16) & 0x7FFF) as f32 / 32768.0;
            val * 2.0 - 1.0
        })
        .collect()
}

fn brute_force_knn(query: &[f32], dataset: &[Vec<f32>], k: usize) -> Vec<u64> {
    let mut distances: Vec<(u64, f32)> = dataset
        .iter()
        .enumerate()
        .map(|(idx, vec)| {
            let sim = simd_native::cosine_similarity_native(query, vec);
            #[allow(clippy::cast_possible_truncation)]
            (idx as u64, sim)
        })
        .collect();

    distances.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    distances.into_iter().take(k).map(|(id, _)| id).collect()
}

/// Comprehensive benchmark with stats output
fn bench_comprehensive(c: &mut Criterion) {
    let configs = [(10_000, 128, "10K/128D"), (100_000, 768, "100K/768D")];

    for (n_vectors, dim, label) in configs {
        println!("\n{}", "=".repeat(60));
        println!("  Configuration: {label} - {n_vectors} vectors, {dim}D");
        println!("{}", "=".repeat(60));

        // Generate dataset
        println!("  Generating {n_vectors} vectors of dimension {dim}...");
        let start = Instant::now();
        let dataset: Vec<Vec<f32>> = (0..n_vectors)
            .map(|i| generate_vector(dim, i as u64))
            .collect();
        println!("  Dataset generated in {:.2?}", start.elapsed());

        // Build index with adaptive params
        println!("  Building HNSW index...");
        let start = Instant::now();
        let params = HnswParams::for_dataset_size(dim, n_vectors);
        println!(
            "  Params: M={}, ef_construction={}",
            params.max_connections, params.ef_construction
        );

        let index = HnswIndex::with_params(dim, DistanceMetric::Cosine, params);
        for (idx, vec) in dataset.iter().enumerate() {
            #[allow(clippy::cast_possible_truncation)]
            index.insert(idx as u64, vec);
        }
        println!("  Index built in {:.2?}", start.elapsed());

        // Generate queries
        let num_queries = 100;
        let queries: Vec<Vec<f32>> = (0..num_queries)
            .map(|i| generate_vector(dim, (n_vectors + i) as u64))
            .collect();

        // Compute ground truth
        println!("  Computing ground truth for {num_queries} queries...");
        let start = Instant::now();
        let ground_truths: Vec<Vec<u64>> = queries
            .iter()
            .map(|q| brute_force_knn(q, &dataset, 10))
            .collect();
        println!("  Ground truth computed in {:.2?}", start.elapsed());

        // Measure recall and latency for each mode
        println!("\n  Results (Native Rust, Criterion):");
        println!("  {}", "-".repeat(55));
        println!(
            "  {:12} {:10} {:12} {:12}",
            "Mode", "ef_search", "Recall@10", "Latency P50"
        );
        println!("  {}", "-".repeat(55));

        for quality in [
            SearchQuality::Fast,
            SearchQuality::Balanced,
            SearchQuality::Accurate,
            SearchQuality::Perfect,
        ] {
            let (quality_name, ef) = match quality {
                SearchQuality::Fast => ("Fast", 64),
                SearchQuality::Balanced => ("Balanced", 128),
                SearchQuality::Accurate => ("Accurate", 256),
                SearchQuality::Perfect => ("Perfect", 2048),
                SearchQuality::Custom(e) => ("Custom", e),
            };

            // Measure latencies
            let mut latencies: Vec<f64> = Vec::with_capacity(num_queries);
            let mut total_recall = 0.0;

            for (query, ground_truth) in queries.iter().zip(&ground_truths) {
                let start = Instant::now();
                let results = index.search_with_quality(query, 10, quality);
                let elapsed = start.elapsed().as_secs_f64() * 1000.0; // ms
                latencies.push(elapsed);

                let result_ids: Vec<u64> = results.iter().map(|(id, _)| *id).collect();
                total_recall += recall_at_k(ground_truth, &result_ids);
            }

            // Calculate P50 latency
            latencies.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let p50_idx = latencies.len() / 2;
            let p50_latency = latencies[p50_idx];

            #[allow(clippy::cast_precision_loss)]
            let avg_recall = total_recall / num_queries as f64 * 100.0;

            let status = if avg_recall >= 95.0 { "✅" } else { "⚠️" };

            println!(
                "  {quality_name:12} {ef:10} {avg_recall:>10.1}% {p50_latency:>10.2}ms {status}"
            );
        }

        println!("  {}", "-".repeat(55));

        // Criterion benchmark for Accurate mode
        let mut group = c.benchmark_group(format!("native_{}", label.replace('/', "_")));
        group.sample_size(20);

        group.bench_function(BenchmarkId::new("search_accurate", label), |b| {
            b.iter(|| {
                for query in &queries {
                    let results = index.search_with_quality(query, 10, SearchQuality::Accurate);
                    black_box(results);
                }
            });
        });

        group.finish();
    }
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(10);
    targets = bench_comprehensive
}
criterion_main!(benches);
