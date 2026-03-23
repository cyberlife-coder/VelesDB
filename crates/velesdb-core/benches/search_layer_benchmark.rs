//! Phase A.0 (#366): Baseline micro-benchmark for HNSW `search_layer` performance.
//!
//! Measures per-query `search()` latency on a pre-built `NativeHnsw` index.
//! `search()` is the public entry point that internally calls `search_layer`
//! (the hot path for both search and insert). By sweeping `ef_search` values
//! we isolate the search-layer traversal cost at different recall budgets.
//!
//! # Run
//!
//! ```bash
//! cargo bench -p velesdb-core --bench search_layer_benchmark -- --noplot
//! ```

#![allow(clippy::cast_precision_loss)]

mod bench_helpers;

use criterion::{
    black_box, criterion_group, criterion_main, measurement::Measurement, BenchmarkGroup,
    BenchmarkId, Criterion, Throughput,
};
use velesdb_core::index::hnsw::native::{CachedSimdDistance, NativeHnsw};
use velesdb_core::DistanceMetric;

/// Number of vectors pre-inserted into the index before benchmarking.
const DATASET_SIZE: usize = 5_000;

/// Number of distinct query vectors (rotated round-robin inside the bench loop).
const NUM_QUERIES: usize = 100;

/// Top-k results requested per search call.
const K: usize = 10;

/// HNSW construction parameters (`M`, `ef_construction`).
const MAX_CONNECTIONS: usize = 16;
const EF_CONSTRUCTION: usize = 200;

/// `ef_search` values to sweep — from lean (50) to generous (256).
const EF_SEARCH_VALUES: &[usize] = &[50, 128, 256];

/// Builds a `NativeHnsw` index pre-populated with `DATASET_SIZE` vectors.
///
/// Uses `CachedSimdDistance` (the non-deprecated, production engine) so the
/// benchmark measures the same code path as real queries.
fn build_index(dim: usize) -> NativeHnsw<CachedSimdDistance> {
    let distance = CachedSimdDistance::new(DistanceMetric::Euclidean, dim);
    let hnsw = NativeHnsw::new(distance, MAX_CONNECTIONS, EF_CONSTRUCTION, DATASET_SIZE);

    for seed in 0..DATASET_SIZE {
        #[allow(clippy::cast_possible_truncation)]
        let v = bench_helpers::generate_vector(dim, seed as u64);
        hnsw.insert(&v).expect("insert during index build");
    }
    hnsw
}

/// Generates `NUM_QUERIES` query vectors for the given dimension.
fn build_queries(dim: usize) -> Vec<Vec<f32>> {
    let base_seed = DATASET_SIZE as u64 + 1; // disjoint from dataset seeds
    (0..NUM_QUERIES)
        .map(|i| {
            #[allow(clippy::cast_possible_truncation)]
            let seed = base_seed + i as u64;
            bench_helpers::generate_vector(dim, seed)
        })
        .collect()
}

/// Benchmarks `NativeHnsw::search()` for a single dimension, sweeping `ef_search`.
fn bench_search_for_dim<M: Measurement>(group: &mut BenchmarkGroup<'_, M>, dim: usize) {
    let index = build_index(dim);
    let queries = build_queries(dim);

    // Throughput = 1 search call per iteration (we measure single-query latency).
    group.throughput(Throughput::Elements(1));

    for &ef_search in EF_SEARCH_VALUES {
        group.bench_with_input(
            BenchmarkId::new(format!("ef{ef_search}"), dim),
            &ef_search,
            |b, &ef| {
                let mut qi = 0_usize;
                b.iter(|| {
                    let q = &queries[qi % NUM_QUERIES];
                    qi = qi.wrapping_add(1);
                    let results = index.search(black_box(q), K, ef);
                    black_box(results);
                });
            },
        );
    }
}

/// Benchmark group: `search_layer/128d` — low-dimension fast path.
fn bench_search_128d(c: &mut Criterion) {
    let mut group = c.benchmark_group("search_layer/128d");
    bench_search_for_dim(&mut group, 128);
    group.finish();
}

/// Benchmark group: `search_layer/768d` — BERT-class dimension.
fn bench_search_768d(c: &mut Criterion) {
    let mut group = c.benchmark_group("search_layer/768d");
    bench_search_for_dim(&mut group, 768);
    group.finish();
}

criterion_group!(benches, bench_search_128d, bench_search_768d);
criterion_main!(benches);
