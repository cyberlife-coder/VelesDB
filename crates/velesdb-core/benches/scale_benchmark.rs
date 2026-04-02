//! Fair-scale benchmarks: 100K vectors at 128 dimensions.
//!
//! These benchmarks use the SAME parameters as competitor benchmarks:
//! - 128 dimensions (SIFT1M standard)
//! - Cosine metric
//! - `ef_search` calibrated per quality mode
//! - Recall measured against brute-force ground truth
//!
//! NO comparison with competitors is made here — only raw `VelesDB` numbers
//! at production-relevant scale.

use criterion::{
    criterion_group, criterion_main, BenchmarkId, Criterion, SamplingMode, Throughput,
};
use std::time::Duration;
use velesdb_core::distance::DistanceMetric;
use velesdb_core::index::hnsw::HnswIndex;
use velesdb_core::{SearchQuality, VectorIndex};

fn generate_vector(dim: usize, seed: u64) -> Vec<f32> {
    let mut v = Vec::with_capacity(dim);
    let mut s = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
    for _ in 0..dim {
        s = s.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        #[allow(clippy::cast_precision_loss)]
        v.push(((s >> 33) as f32) / (u32::MAX as f32) - 0.5);
    }
    // Normalize for cosine
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in &mut v {
            *x /= norm;
        }
    }
    v
}

#[allow(clippy::too_many_lines, clippy::cast_possible_truncation)]
fn bench_scale(c: &mut Criterion) {
    let dim = 128_usize;
    let k = 10_usize;
    let n_queries = 50_usize;

    for &n_vectors in &[100_000_u64] {
        let mut group = c.benchmark_group(format!("scale_{n_vectors}"));
        group.sample_size(10);
        group.sampling_mode(SamplingMode::Flat);
        group.measurement_time(Duration::from_secs(30));

        // --- INSERT sequential benchmark (baseline) ---
        group.throughput(Throughput::Elements(n_vectors));
        group.bench_function(
            BenchmarkId::new("insert_sequential", format!("{n_vectors}x{dim}d")),
            |b| {
                b.iter_with_setup(
                    || {
                        let vectors: Vec<Vec<f32>> =
                            (0..n_vectors).map(|i| generate_vector(dim, i)).collect();
                        (
                            HnswIndex::new(dim, DistanceMetric::Cosine)
                                .expect("bench: create index"),
                            vectors,
                        )
                    },
                    |(index, vectors)| {
                        for (i, v) in vectors.iter().enumerate() {
                            index.insert(i as u64, v);
                        }
                    },
                );
            },
        );

        // --- INSERT parallel benchmark (production path) ---
        group.bench_function(
            BenchmarkId::new("insert_parallel", format!("{n_vectors}x{dim}d")),
            |b| {
                b.iter_with_setup(
                    || {
                        let vectors: Vec<Vec<f32>> =
                            (0..n_vectors).map(|i| generate_vector(dim, i)).collect();
                        (
                            HnswIndex::new(dim, DistanceMetric::Cosine)
                                .expect("bench: create index"),
                            vectors,
                        )
                    },
                    |(index, vectors)| {
                        let batch: Vec<(u64, &[f32])> = vectors
                            .iter()
                            .enumerate()
                            .map(|(i, v)| (i as u64, v.as_slice()))
                            .collect();
                        index.insert_batch_parallel(batch);
                    },
                );
            },
        );

        // Build index for search benchmarks (using batch insert for realistic graph)
        let index = HnswIndex::new(dim, DistanceMetric::Cosine).expect("bench: create index");
        let vectors: Vec<Vec<f32>> = (0..n_vectors).map(|i| generate_vector(dim, i)).collect();
        let batch: Vec<(u64, &[f32])> = vectors
            .iter()
            .enumerate()
            .map(|(i, v)| (i as u64, v.as_slice()))
            .collect();
        index.insert_batch_parallel(batch);

        let queries: Vec<Vec<f32>> = (0..n_queries)
            .map(|i| generate_vector(dim, n_vectors + i as u64))
            .collect();

        // --- SEARCH benchmarks per quality mode ---
        group.throughput(Throughput::Elements(n_queries as u64));
        for (mode_name, quality) in [
            ("fast", SearchQuality::Fast),
            ("balanced", SearchQuality::Balanced),
            ("accurate", SearchQuality::Accurate),
        ] {
            group.bench_function(
                BenchmarkId::new("search", format!("{mode_name}_k{k}")),
                |b| {
                    b.iter(|| {
                        for query in &queries {
                            let _ = index.search_with_quality(query, k, quality).unwrap();
                        }
                    });
                },
            );
        }

        // --- RECALL measurement ---
        let ground_truths: Vec<Vec<u64>> = queries
            .iter()
            .map(|query| {
                let mut distances: Vec<(u64, f32)> = vectors
                    .iter()
                    .enumerate()
                    .map(|(i, v)| {
                        let dist =
                            1.0 - velesdb_core::simd_native::cosine_similarity_native(query, v);
                        (i as u64, dist)
                    })
                    .collect();
                distances
                    .sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
                distances.iter().take(k).map(|(id, _)| *id).collect()
            })
            .collect();

        for (mode_name, quality) in [
            ("fast", SearchQuality::Fast),
            ("balanced", SearchQuality::Balanced),
            ("accurate", SearchQuality::Accurate),
        ] {
            let ef = quality.ef_search_for_scale(k, n_vectors as usize);
            let mut total_recall = 0.0_f64;
            for (qi, query) in queries.iter().enumerate() {
                let results: Vec<u64> = index
                    .search_with_quality(query, k, quality)
                    .unwrap()
                    .iter()
                    .map(|r| r.id)
                    .collect();
                let hits = results
                    .iter()
                    .filter(|id| ground_truths[qi].contains(id))
                    .count();
                #[allow(clippy::cast_precision_loss)]
                {
                    total_recall += hits as f64 / k as f64;
                }
            }
            #[allow(clippy::cast_precision_loss)]
            let avg_recall = total_recall / n_queries as f64;
            println!(
                "  {mode_name} (ef={ef}): recall@{k} = {:.1}%",
                avg_recall * 100.0
            );
        }

        // --- THROUGHPUT: QPS measurement ---
        group.throughput(Throughput::Elements(n_queries as u64));
        group.bench_function(
            BenchmarkId::new("throughput", format!("{n_queries}q_balanced")),
            |b| {
                b.iter(|| {
                    for query in &queries {
                        let _ = index
                            .search_with_quality(query, k, SearchQuality::Balanced)
                            .unwrap();
                    }
                });
            },
        );

        group.finish();
    }
}

criterion_group!(benches, bench_scale);
criterion_main!(benches);
