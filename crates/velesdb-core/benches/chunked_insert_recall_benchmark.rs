//! Chunked insert recall quality benchmark for `VelesDB` HNSW index.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::unreadable_literal
)]
//!
//! Run with: `cargo bench --bench chunked_insert_recall_benchmark`
//!
//! This benchmark measures **search quality** (recall@10) after bulk insertion
//! via `insert_batch_parallel` (chunked) vs sequential `insert`, ensuring that
//! the parallel path does not degrade recall below an acceptable threshold.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use velesdb_core::distance::DistanceMetric;
use velesdb_core::metrics::recall_at_k;
use velesdb_core::{HnswIndex, HnswParams, SearchQuality, VectorIndex};

const DIM: usize = 128;
const DATASET_SIZE: usize = 5000;
const NUM_QUERIES: usize = 100;
const K: usize = 10;
const MIN_PARALLEL_RECALL: f64 = 0.85;

/// Generate a deterministic vector using a simple LCG seeded by index.
fn generate_vector(dim: usize, seed: u64) -> Vec<f32> {
    let mut state = seed;
    (0..dim)
        .map(|_| {
            state = state.wrapping_mul(1103515245).wrapping_add(12345);
            let val = ((state >> 16) & 0x7FFF) as f32 / 32768.0;
            val * 2.0 - 1.0 // Range [-1, 1]
        })
        .collect()
}

/// Compute exact k-nearest neighbors via brute-force cosine similarity.
fn brute_force_knn(query: &[f32], dataset: &[Vec<f32>], k: usize) -> Vec<u64> {
    let mut distances: Vec<(u64, f32)> = dataset
        .iter()
        .enumerate()
        .map(|(idx, vec)| {
            let sim = velesdb_core::simd_native::cosine_similarity_native(query, vec);
            (idx as u64, sim)
        })
        .collect();

    // Cosine similarity: higher is better
    distances.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    distances.into_iter().take(k).map(|(id, _)| id).collect()
}

/// Build an HNSW index via sequential single-vector inserts.
fn build_sequential(dataset: &[(u64, Vec<f32>)]) -> HnswIndex {
    let params = HnswParams::custom(16, 200, 10_000);
    let index = HnswIndex::with_params(DIM, DistanceMetric::Cosine, params)
        .expect("Failed to create sequential HNSW index");
    for (id, vec) in dataset {
        index.insert(*id, vec);
    }
    index
}

/// Build an HNSW index via chunked parallel batch insert.
fn build_parallel(dataset: &[(u64, Vec<f32>)]) -> HnswIndex {
    let params = HnswParams::custom(16, 200, 10_000);
    let index = HnswIndex::with_params(DIM, DistanceMetric::Cosine, params)
        .expect("Failed to create parallel HNSW index");
    index.insert_batch_parallel(dataset.iter().map(|(id, v)| (*id, v.as_slice())));
    index.set_searching_mode();
    index
}

/// Compute mean recall@k across all queries for a given index.
fn measure_recall(index: &HnswIndex, queries: &[Vec<f32>], ground_truths: &[Vec<u64>]) -> f64 {
    let total: f64 = queries
        .iter()
        .zip(ground_truths)
        .map(|(query, gt)| {
            let results = index.search_with_quality(query, K, SearchQuality::Balanced);
            let result_ids: Vec<u64> = results.iter().map(|sr| sr.id).collect();
            recall_at_k(gt, &result_ids)
        })
        .sum();

    total / queries.len() as f64
}

/// One-shot quality measurement: parallel vs sequential recall comparison.
fn bench_chunked_insert_recall(c: &mut Criterion) {
    let mut group = c.benchmark_group("chunked_insert_recall");
    group.sample_size(10);

    // 1. Generate deterministic dataset (index-based seeding)
    let dataset: Vec<(u64, Vec<f32>)> = (0..DATASET_SIZE)
        .map(|i| (i as u64, generate_vector(DIM, i as u64)))
        .collect();
    let raw_vectors: Vec<Vec<f32>> = dataset.iter().map(|(_, v)| v.clone()).collect();

    // 2. Generate deterministic query vectors (seeds offset past dataset)
    let queries: Vec<Vec<f32>> = (0..NUM_QUERIES)
        .map(|i| generate_vector(DIM, (DATASET_SIZE + i) as u64))
        .collect();

    // 3. Compute brute-force ground truth
    let ground_truths: Vec<Vec<u64>> = queries
        .iter()
        .map(|q| brute_force_knn(q, &raw_vectors, K))
        .collect();

    // 4. Build both indexes
    let sequential_index = build_sequential(&dataset);
    let parallel_index = build_parallel(&dataset);

    // 5. Measure recall (one-shot, then benchmark the measurement itself)
    let sequential_recall = measure_recall(&sequential_index, &queries, &ground_truths);
    let parallel_recall = measure_recall(&parallel_index, &queries, &ground_truths);
    let delta = parallel_recall - sequential_recall;

    println!("\n=== Chunked Insert Recall@{K} (n={DATASET_SIZE}, dim={DIM}, M=16, ef_c=200) ===");
    println!("Sequential recall:  {:.1}%", sequential_recall * 100.0);
    println!("Parallel recall:    {:.1}%", parallel_recall * 100.0);
    println!("Delta (par - seq):  {:+.2}%", delta * 100.0);

    assert!(
        parallel_recall >= MIN_PARALLEL_RECALL,
        "Parallel recall {parallel_recall:.3} is below threshold {MIN_PARALLEL_RECALL}"
    );

    // Wrap the measurement in a criterion bench so the harness is satisfied
    group.bench_function("parallel_recall_measurement", |b| {
        b.iter(|| black_box(measure_recall(&parallel_index, &queries, &ground_truths)));
    });

    group.bench_function("sequential_recall_measurement", |b| {
        b.iter(|| black_box(measure_recall(&sequential_index, &queries, &ground_truths)));
    });

    group.finish();
}

criterion_group!(benches, bench_chunked_insert_recall);
criterion_main!(benches);
