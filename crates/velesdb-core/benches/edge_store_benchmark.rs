//! Benchmarks for EdgeStore and ConcurrentEdgeStore performance.
//!
//! Run with: cargo bench --package velesdb-core edge_store

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use std::sync::Arc;
use std::thread;
use velesdb_core::collection::graph::{ConcurrentEdgeStore, EdgeStore, GraphEdge};

fn create_edge_store_with_edges(num_nodes: u64, avg_degree: u64) -> EdgeStore {
    let mut store = EdgeStore::new();
    let mut edge_id = 0u64;

    for node in 0..num_nodes {
        for i in 0..avg_degree {
            let target = (node + i + 1) % num_nodes;
            store.add_edge(GraphEdge::new(edge_id, node, target, "LINK"));
            edge_id += 1;
        }
    }
    store
}

fn create_concurrent_store_with_edges(num_nodes: u64, avg_degree: u64) -> ConcurrentEdgeStore {
    let store = ConcurrentEdgeStore::new();
    let mut edge_id = 0u64;

    for node in 0..num_nodes {
        for i in 0..avg_degree {
            let target = (node + i + 1) % num_nodes;
            store.add_edge(GraphEdge::new(edge_id, node, target, "LINK"));
            edge_id += 1;
        }
    }
    store
}

fn bench_get_neighbors(c: &mut Criterion) {
    let mut group = c.benchmark_group("EdgeStore::get_neighbors");

    for degree in [5, 10, 50].iter() {
        let store = create_edge_store_with_edges(1000, *degree);

        group.bench_with_input(BenchmarkId::new("degree", degree), degree, |b, _| {
            b.iter(|| black_box(store.get_outgoing(42)))
        });
    }
    group.finish();
}

fn bench_add_edge(c: &mut Criterion) {
    c.bench_function("EdgeStore::add_edge", |b| {
        let mut store = EdgeStore::new();
        let mut id = 0u64;

        b.iter(|| {
            store.add_edge(GraphEdge::new(id, id % 1000, (id + 1) % 1000, "LINK"));
            id += 1;
        })
    });
}

fn bench_cascade_delete(c: &mut Criterion) {
    let mut group = c.benchmark_group("EdgeStore::cascade_delete");

    for degree in [10, 50, 100].iter() {
        group.bench_with_input(BenchmarkId::new("degree", degree), degree, |b, &degree| {
            b.iter_batched(
                || {
                    // Setup: create store with node 0 having many edges
                    let mut store = EdgeStore::new();
                    for i in 0..degree {
                        store.add_edge(GraphEdge::new(i, 0, i + 1, "OUT"));
                        store.add_edge(GraphEdge::new(degree + i, i + 1, 0, "IN"));
                    }
                    store
                },
                |mut store| {
                    store.remove_node_edges(0);
                },
                criterion::BatchSize::SmallInput,
            )
        });
    }
    group.finish();
}

fn bench_concurrent_get_neighbors(c: &mut Criterion) {
    let store = Arc::new(create_concurrent_store_with_edges(1000, 10));

    c.bench_function("ConcurrentEdgeStore::get_neighbors", |b| {
        b.iter(|| black_box(store.get_neighbors(42)))
    });
}

fn bench_concurrent_add_edge(c: &mut Criterion) {
    c.bench_function("ConcurrentEdgeStore::add_edge", |b| {
        let store = ConcurrentEdgeStore::new();
        let mut id = 0u64;

        b.iter(|| {
            store.add_edge(GraphEdge::new(id, id % 1000, (id + 1) % 1000, "LINK"));
            id += 1;
        })
    });
}

fn bench_traverse_bfs(c: &mut Criterion) {
    let mut group = c.benchmark_group("ConcurrentEdgeStore::traverse_bfs");

    for depth in [1, 2, 3].iter() {
        let store = Arc::new(create_concurrent_store_with_edges(1000, 5));

        group.bench_with_input(BenchmarkId::new("depth", depth), depth, |b, &depth| {
            b.iter(|| black_box(store.traverse_bfs(0, depth)))
        });
    }
    group.finish();
}

fn bench_concurrent_parallel_reads(c: &mut Criterion) {
    let store = Arc::new(create_concurrent_store_with_edges(1000, 10));

    c.bench_function("ConcurrentEdgeStore::parallel_reads_8_threads", |b| {
        b.iter(|| {
            let handles: Vec<_> = (0..8)
                .map(|t| {
                    let store_clone = Arc::clone(&store);
                    thread::spawn(move || {
                        for i in 0..100 {
                            black_box(store_clone.get_neighbors((t * 100 + i) % 1000));
                        }
                    })
                })
                .collect();

            for h in handles {
                h.join().unwrap();
            }
        })
    });
}

criterion_group!(
    benches,
    bench_get_neighbors,
    bench_add_edge,
    bench_cascade_delete,
    bench_concurrent_get_neighbors,
    bench_concurrent_add_edge,
    bench_traverse_bfs,
    bench_concurrent_parallel_reads,
);
criterion_main!(benches);
