//! Criterion benchmarks for graph traversal v2 (CSR snapshot).
//!
//! Run with: `cargo bench --package velesdb-core --bench graph_traversal_v2`

#![allow(clippy::cast_possible_truncation)]

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use rustc_hash::FxHashSet;
use velesdb_core::collection::graph::{
    bfs_traverse_csr, bfs_traverse_csr_filtered, CsrSnapshot, EdgeStore, GraphEdge, LabelFilter,
    LabelId, LabelTable, NoFilter, TraversalConfig,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Builds an `EdgeStore` (with CSR snapshot) for the given graph size.
///
/// Labels cycle through 5 fixed strings so that label-based filtering
/// benchmarks have realistic selectivity.
fn build_stores(num_nodes: u64, degree: u64) -> (EdgeStore, LabelTable) {
    let labels = ["KNOWS", "FOLLOWS", "LIKES", "WORKS_AT", "CREATED"];
    let mut store = EdgeStore::new();
    let label_table = LabelTable::new();
    let mut eid = 0u64;

    for src in 0..num_nodes {
        for d in 0..degree {
            let tgt = (src + d + 1) % num_nodes;
            let label = labels[(eid as usize) % labels.len()];
            if let Ok(edge) = GraphEdge::new(eid, src, tgt, label) {
                let _ = store.add_edge(edge);
            }
            eid += 1;
        }
    }
    (store, label_table)
}

/// Builds a `CsrSnapshot` and the backing `EdgeStore`.
fn build_snapshot(num_nodes: u64, degree: u64) -> (CsrSnapshot, EdgeStore) {
    let (mut store, _label_table) = build_stores(num_nodes, degree);
    store.build_read_snapshot();
    let snapshot = store.csr_snapshot().expect("snapshot built").clone();
    (snapshot, store)
}

// ---------------------------------------------------------------------------
// Benchmark: BFS EdgeStore vs CsrSnapshot
// ---------------------------------------------------------------------------

fn bench_bfs_edgestore_vs_csr(c: &mut Criterion) {
    let mut group = c.benchmark_group("bfs_edgestore_vs_csr");
    let degree = 5u64;
    let depth = 3u32;

    for &num_nodes in &[1_000u64, 10_000, 100_000] {
        let (snapshot, store) = build_snapshot(num_nodes, degree);
        let config = TraversalConfig::with_range(1, depth);

        group.bench_with_input(
            BenchmarkId::new("csr", num_nodes),
            &num_nodes,
            |b, _| {
                b.iter(|| bfs_traverse_csr(black_box(&snapshot), 0, black_box(&config)));
            },
        );

        // EdgeStore BFS via the CSR snapshot path (store has snapshot built)
        group.bench_with_input(
            BenchmarkId::new("edgestore_csr", num_nodes),
            &num_nodes,
            |b, _| {
                let snap = store.csr_snapshot().expect("snapshot");
                b.iter(|| bfs_traverse_csr(black_box(snap), 0, black_box(&config)));
            },
        );
    }
    group.finish();
}

// ---------------------------------------------------------------------------
// Benchmark: BFS CSR with predicate pushdown
// ---------------------------------------------------------------------------

fn bench_bfs_csr_with_predicate(c: &mut Criterion) {
    let mut group = c.benchmark_group("bfs_csr_with_predicate");
    let num_nodes = 10_000u64;
    let degree = 5u64;
    let depth = 3u32;

    let (snapshot, _store) = build_snapshot(num_nodes, degree);
    let config = TraversalConfig::with_range(1, depth);

    // Selectivity via label subsets (5 labels total, cycled uniformly)
    let selectivities: &[(&str, &[u32])] = &[
        ("1_of_5", &[0]),
        ("2_of_5", &[0, 1]),
        ("3_of_5", &[0, 1, 2]),
    ];

    for &(name, label_ids) in selectivities {
        let mut allowed = FxHashSet::default();
        for &lid in label_ids {
            allowed.insert(LabelId::from_u32(lid));
        }
        let predicate = LabelFilter::new(allowed);

        group.bench_with_input(BenchmarkId::new("filtered", name), &name, |b, _| {
            b.iter(|| {
                bfs_traverse_csr_filtered(
                    black_box(&snapshot),
                    0,
                    black_box(&config),
                    black_box(&predicate),
                )
            });
        });
    }

    // Baseline: NoFilter
    let no_filter = NoFilter;
    group.bench_function("no_filter", |b| {
        b.iter(|| {
            bfs_traverse_csr_filtered(
                black_box(&snapshot),
                0,
                black_box(&config),
                black_box(&no_filter),
            )
        });
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// Benchmark: CSR build time
// ---------------------------------------------------------------------------

fn bench_csr_build_time(c: &mut Criterion) {
    let mut group = c.benchmark_group("csr_build_time");
    let degree = 5u64;

    for &num_nodes in &[1_000u64, 10_000] {
        let (mut store, _label_table) = build_stores(num_nodes, degree);

        group.bench_with_input(
            BenchmarkId::new("build", num_nodes),
            &num_nodes,
            |b, _| {
                b.iter(|| {
                    store.build_read_snapshot();
                    black_box(store.csr_snapshot());
                });
            },
        );
    }
    group.finish();
}

// ---------------------------------------------------------------------------
// Benchmark: BFS on dense graph (degree 20)
// ---------------------------------------------------------------------------

fn bench_bfs_dense_graph(c: &mut Criterion) {
    let mut group = c.benchmark_group("bfs_dense_graph");
    let num_nodes = 10_000u64;
    let degree = 20u64;
    let depth = 3u32;

    let (snapshot, _store) = build_snapshot(num_nodes, degree);
    let config = TraversalConfig::with_range(1, depth);

    group.bench_function("csr_10k_deg20", |b| {
        b.iter(|| bfs_traverse_csr(black_box(&snapshot), 0, black_box(&config)));
    });

    // Filtered traversal on dense graph (1-of-5 selectivity)
    let mut allowed = FxHashSet::default();
    allowed.insert(LabelId::from_u32(0));
    let predicate = LabelFilter::new(allowed);

    group.bench_function("filtered_10k_deg20_1of5", |b| {
        b.iter(|| {
            bfs_traverse_csr_filtered(
                black_box(&snapshot),
                0,
                black_box(&config),
                black_box(&predicate),
            )
        });
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// Benchmark: Edge mutation throughput with lazy CSR rebuild
// ---------------------------------------------------------------------------

/// Measures `add_edge` throughput on `ConcurrentEdgeStore` with lazy CSR.
///
/// The lazy rebuild (via `csr_dirty` flag) defers the O(N+E) snapshot
/// rebuild to the next read, so pure write throughput should be high.
fn bench_add_edge_throughput(c: &mut Criterion) {
    use velesdb_core::collection::graph::ConcurrentEdgeStore;

    let mut group = c.benchmark_group("add_edge_throughput");
    let labels = ["KNOWS", "FOLLOWS", "LIKES", "WORKS_AT", "CREATED"];

    for &batch_size in &[1_000u64, 10_000] {
        group.bench_with_input(
            BenchmarkId::new("lazy_csr", batch_size),
            &batch_size,
            |b, &n| {
                b.iter_with_setup(
                    || ConcurrentEdgeStore::with_shards(4),
                    |store| {
                        for eid in 0..n {
                            let src = eid % 500;
                            let tgt = (eid + 1) % 500;
                            let label = labels[(eid as usize) % labels.len()];
                            if let Ok(edge) = GraphEdge::new(eid, src, tgt, label) {
                                let _ = store.add_edge(edge);
                            }
                        }
                        black_box(&store);
                    },
                );
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_bfs_edgestore_vs_csr,
    bench_bfs_csr_with_predicate,
    bench_csr_build_time,
    bench_bfs_dense_graph,
    bench_add_edge_throughput,
);
criterion_main!(benches);
