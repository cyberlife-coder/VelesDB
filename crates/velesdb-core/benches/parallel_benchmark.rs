//! Parallel search benchmark for `VelesDB` multi-threading validation.
#![allow(
    clippy::cast_precision_loss,
    clippy::redundant_closure_for_method_calls,
    clippy::unreadable_literal
)]
//!
//! Run with: `cargo bench --bench parallel_benchmark`
//!
//! Measures scaling efficiency across multiple CPU cores.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use velesdb_core::distance::DistanceMetric;
use velesdb_core::{HnswIndex, HnswParams, SearchQuality, VectorIndex};

/// Generate a deterministic vector
fn generate_vector(dim: usize, seed: u64) -> Vec<f32> {
    let mut state = seed;
    (0..dim)
        .map(|_| {
            state = state.wrapping_mul(1103515245).wrapping_add(12345);
            #[allow(clippy::cast_precision_loss)]
            let val = ((state >> 16) & 0x7FFF) as f32 / 32768.0;
            val * 2.0 - 1.0
        })
        .collect()
}

/// Benchmark batch search: sequential vs parallel
fn bench_batch_search(c: &mut Criterion) {
    let mut group = c.benchmark_group("batch_search");
    group.sample_size(20);

    let dim = 128;
    let n = 10000;
    let num_queries = 100;
    let k = 10;

    // Generate dataset
    let dataset: Vec<Vec<f32>> = (0..n).map(|i| generate_vector(dim, i as u64)).collect();

    // Build HNSW index
    let params = HnswParams::custom(16, 200, n + 1000);
    let index = HnswIndex::with_params(dim, DistanceMetric::Cosine, params);

    for (idx, vec) in dataset.iter().enumerate() {
        #[allow(clippy::cast_possible_truncation)]
        index.insert(idx as u64, vec);
    }

    // Generate queries
    let queries: Vec<Vec<f32>> = (0..num_queries)
        .map(|i| generate_vector(dim, (n + i) as u64))
        .collect();
    let query_refs: Vec<&[f32]> = queries.iter().map(|q| q.as_slice()).collect();

    // Sequential batch search
    group.bench_function(
        BenchmarkId::new("sequential", format!("{num_queries}q")),
        |b| {
            b.iter(|| {
                let results: Vec<Vec<(u64, f32)>> = queries
                    .iter()
                    .map(|q| index.search_with_quality(q, k, SearchQuality::Balanced))
                    .collect();
                black_box(results)
            });
        },
    );

    // Parallel batch search
    group.bench_function(
        BenchmarkId::new("parallel", format!("{num_queries}q")),
        |b| {
            b.iter(|| {
                let results = index.search_batch_parallel(&query_refs, k, SearchQuality::Balanced);
                black_box(results)
            });
        },
    );

    group.finish();
}

/// Benchmark brute force: single-thread vs parallel
fn bench_brute_force(c: &mut Criterion) {
    let mut group = c.benchmark_group("brute_force");
    group.sample_size(20);

    let dim = 128;
    let k = 10;

    for &n in &[1000, 10000, 50000] {
        // Generate dataset
        let dataset: Vec<Vec<f32>> = (0..n).map(|i| generate_vector(dim, i as u64)).collect();

        // Build HNSW index (for brute_force_search_parallel)
        let params = HnswParams::custom(16, 200, n + 1000);
        let index = HnswIndex::with_params(dim, DistanceMetric::Cosine, params);

        for (idx, vec) in dataset.iter().enumerate() {
            #[allow(clippy::cast_possible_truncation)]
            index.insert(idx as u64, vec);
        }

        let query = generate_vector(dim, n as u64);

        // Parallel brute force
        group.bench_function(BenchmarkId::new("parallel", format!("{n}v")), |b| {
            b.iter(|| {
                let results = index.brute_force_search_parallel(&query, k);
                black_box(results)
            });
        });
    }

    group.finish();
}

/// Benchmark parallel insert (WIS-9: lock contention reduction)
///
/// Measures throughput of `insert_batch_parallel` which benefits from
/// the `HnswMappings` refactoring (4 locks → 2 locks per insert).
fn bench_parallel_insert(c: &mut Criterion) {
    let mut group = c.benchmark_group("parallel_insert");
    group.sample_size(10);

    let dim = 128;

    for &n in &[1000, 5000, 10000] {
        // Generate vectors to insert
        let vectors: Vec<(u64, Vec<f32>)> = (0..n)
            .map(|i| {
                #[allow(clippy::cast_possible_truncation)]
                (i as u64, generate_vector(dim, i as u64))
            })
            .collect();

        group.bench_function(
            criterion::BenchmarkId::new("batch_parallel", format!("{n}v")),
            |b| {
                b.iter_with_setup(
                    || {
                        // Setup: create fresh index for each iteration
                        let params = HnswParams::custom(16, 200, n + 1000);
                        let index = HnswIndex::with_params(dim, DistanceMetric::Cosine, params);
                        (index, vectors.clone())
                    },
                    |(index, vecs)| {
                        // Measure: parallel batch insert
                        let inserted = index.insert_batch_parallel(vecs);
                        index.set_searching_mode();
                        black_box(inserted)
                    },
                );
            },
        );

        // Compare with sequential insert
        group.bench_function(
            criterion::BenchmarkId::new("sequential", format!("{n}v")),
            |b| {
                b.iter_with_setup(
                    || {
                        let params = HnswParams::custom(16, 200, n + 1000);
                        let index = HnswIndex::with_params(dim, DistanceMetric::Cosine, params);
                        (index, vectors.clone())
                    },
                    |(index, vecs)| {
                        for (id, vec) in vecs {
                            index.insert(id, &vec);
                        }
                        black_box(index.len())
                    },
                );
            },
        );
    }

    group.finish();
}

/// Benchmark scaling with thread count
fn bench_thread_scaling(c: &mut Criterion) {
    use rayon::ThreadPoolBuilder;

    let mut group = c.benchmark_group("thread_scaling");
    group.sample_size(10);

    let dim = 128;
    let n = 50000;
    let k = 10;

    // Generate dataset
    let dataset: Vec<Vec<f32>> = (0..n).map(|i| generate_vector(dim, i as u64)).collect();

    // Build index
    let params = HnswParams::custom(16, 200, n + 1000);
    let index = HnswIndex::with_params(dim, DistanceMetric::Cosine, params);

    for (idx, vec) in dataset.iter().enumerate() {
        #[allow(clippy::cast_possible_truncation)]
        index.insert(idx as u64, vec);
    }

    let query = generate_vector(dim, n as u64);

    // Test with different thread counts
    for &threads in &[1, 2, 4, 8] {
        let pool = ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .expect("Failed to create thread pool");

        group.bench_function(
            BenchmarkId::new("brute_force", format!("{threads}t")),
            |b| {
                b.iter(|| {
                    pool.install(|| {
                        let results = index.brute_force_search_parallel(&query, k);
                        black_box(results)
                    })
                });
            },
        );
    }

    group.finish();
}

// =============================================================================
// EPIC-051 US-005: Graph Traversal Benchmarks
// =============================================================================

use std::collections::HashMap;
use velesdb_core::collection::search::query::parallel_traversal::{
    FrontierParallelBFS, ParallelConfig, ParallelTraverser, ShardedTraverser,
};

/// Generate a random graph with given size and average degree
fn generate_graph(num_nodes: usize, avg_degree: usize, seed: u64) -> HashMap<u64, Vec<(u64, u64)>> {
    let mut graph = HashMap::new();
    let mut state = seed;

    for node in 0..num_nodes {
        let mut edges = Vec::new();
        for _ in 0..avg_degree {
            state = state.wrapping_mul(1103515245).wrapping_add(12345);
            #[allow(clippy::cast_possible_truncation)]
            let target = ((state >> 16) as usize) % num_nodes;
            let edge_id = (node * 1000 + edges.len()) as u64;
            edges.push((target as u64, edge_id));
        }
        graph.insert(node as u64, edges);
    }
    graph
}

/// Benchmark: Sequential vs Parallel BFS from multiple start nodes
fn bench_parallel_bfs(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_parallel_bfs");
    group.sample_size(20);

    for &size in &[1_000, 10_000] {
        let graph = generate_graph(size, 5, 42);
        let start_nodes: Vec<u64> = (0..100).collect();

        // Sequential BFS (threshold very high)
        let seq_config = ParallelConfig::new()
            .with_max_depth(3)
            .with_parallel_threshold(100_000) // Force sequential
            .with_limit(1000);
        let seq_traverser = ParallelTraverser::with_config(seq_config);

        group.bench_function(BenchmarkId::new("sequential", format!("{size}n")), |b| {
            let graph_ref = &graph;
            b.iter(|| {
                let get_neighbors = |node: u64| -> Vec<(u64, u64)> {
                    graph_ref.get(&node).cloned().unwrap_or_default()
                };
                let (results, _) = seq_traverser.bfs_parallel(&start_nodes, get_neighbors);
                black_box(results)
            });
        });

        // Parallel BFS
        let par_config = ParallelConfig::new()
            .with_max_depth(3)
            .with_parallel_threshold(10)
            .with_limit(1000);
        let par_traverser = ParallelTraverser::with_config(par_config);

        group.bench_function(BenchmarkId::new("parallel", format!("{size}n")), |b| {
            let graph_ref = &graph;
            b.iter(|| {
                let get_neighbors = |node: u64| -> Vec<(u64, u64)> {
                    graph_ref.get(&node).cloned().unwrap_or_default()
                };
                let (results, _) = par_traverser.bfs_parallel(&start_nodes, get_neighbors);
                black_box(results)
            });
        });
    }

    group.finish();
}

/// Benchmark: Frontier-parallel BFS (single start, wide fanout)
fn bench_frontier_parallel_bfs(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_frontier_bfs");
    group.sample_size(20);

    // High-degree graph (average 20 neighbors)
    let graph = generate_graph(10_000, 20, 42);

    // Sequential frontier expansion
    let seq_config = ParallelConfig::new()
        .with_max_depth(4)
        .with_min_frontier(100_000) // Force sequential
        .with_limit(5000);
    let seq_bfs = FrontierParallelBFS::with_config(seq_config);

    group.bench_function("sequential_frontier", |b| {
        let graph_ref = &graph;
        b.iter(|| {
            let get_neighbors = |node: u64| -> Vec<(u64, u64)> {
                graph_ref.get(&node).cloned().unwrap_or_default()
            };
            let (results, _) = seq_bfs.traverse(0, get_neighbors);
            black_box(results)
        });
    });

    // Parallel frontier expansion
    let par_config = ParallelConfig::new()
        .with_max_depth(4)
        .with_min_frontier(10)
        .with_limit(5000);
    let par_bfs = FrontierParallelBFS::with_config(par_config);

    group.bench_function("parallel_frontier", |b| {
        let graph_ref = &graph;
        b.iter(|| {
            let get_neighbors = |node: u64| -> Vec<(u64, u64)> {
                graph_ref.get(&node).cloned().unwrap_or_default()
            };
            let (results, _) = par_bfs.traverse(0, get_neighbors);
            black_box(results)
        });
    });

    group.finish();
}

/// Benchmark: Shard-parallel traversal scaling
fn bench_shard_parallel(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_shard_parallel");
    group.sample_size(20);

    let graph = generate_graph(10_000, 5, 42);
    let start_nodes: Vec<u64> = (0..200).collect();

    for &num_shards in &[1, 2, 4, 8] {
        let config = ParallelConfig::new()
            .with_max_depth(3)
            .with_parallel_threshold(10)
            .with_limit(2000);
        let traverser = ShardedTraverser::with_config(num_shards, config);

        group.bench_function(BenchmarkId::new("shards", format!("{num_shards}s")), |b| {
            let graph_ref = &graph;
            b.iter(|| {
                let get_neighbors = |node: u64| -> Vec<(u64, u64)> {
                    graph_ref.get(&node).cloned().unwrap_or_default()
                };
                let (results, _) = traverser.traverse_parallel(&start_nodes, get_neighbors);
                black_box(results)
            });
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_batch_search,
    bench_brute_force,
    bench_parallel_insert,
    bench_thread_scaling,
    bench_parallel_bfs,
    bench_frontier_parallel_bfs,
    bench_shard_parallel
);
criterion_main!(benches);
