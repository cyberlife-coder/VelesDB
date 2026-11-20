//! A search racing vector overwrites of existing nodes never comes back empty.

use super::*;
use crate::collection::graph::GraphSchema;
use crate::distance::DistanceMetric;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

const DIM: usize = 64;
const NODES: u64 = 1_000;
const OVERWRITES: u64 = 200;
const K: usize = 10;

/// A pseudo-random `DIM`-d vector, the same for the same `seed`.
fn seeded_vector(seed: u64) -> Vec<f32> {
    let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    (0..DIM)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let unit = f32::from(u16::try_from(state % 10_000).expect("below 10 000")) / 10_000.0;
            unit - 0.5
        })
        .collect()
}

/// Overwriting a node's embedding leaves every other node untouched, so a
/// k-NN search racing it finds `k` of them, the overwritten node's old or
/// new vector aside. The searchers query the vector being written: the
/// greedy descent then lands on the node the overwrite is linking whenever
/// the graph lets a search reach it before its layer-0 links exist, which
/// once returned an empty result.
#[test]
fn test_a_search_racing_embedding_overwrites_never_comes_back_empty() {
    let dir = tempfile::tempdir().expect("tempdir");
    let col = GraphCollection::create(
        dir.path().to_path_buf(),
        "kg",
        Some(DIM),
        DistanceMetric::Cosine,
        GraphSchema::schemaless(),
    )
    .expect("create");
    let payload = serde_json::json!({"kind": "node"});
    for id in 0..NODES {
        col.upsert_node(id, &payload, Some(seeded_vector(id)))
            .expect("prefill");
    }
    let writing = parking_lot::Mutex::new(seeded_vector(0));
    let done = AtomicBool::new(false);
    let short = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for _ in 0..3 {
            scope.spawn(|| {
                while !done.load(Ordering::Acquire) {
                    let query = writing.lock().clone();
                    if col.search(&query, K).expect("search").len() < K {
                        short.fetch_add(1, Ordering::Relaxed);
                    }
                }
            });
        }
        for n in 0..OVERWRITES {
            let vector = seeded_vector(NODES + n);
            writing.lock().clone_from(&vector);
            col.upsert_node((n * 7) % NODES, &payload, Some(vector))
                .expect("overwrite");
        }
        done.store(true, Ordering::Release);
    });
    assert_eq!(
        short.load(Ordering::Relaxed),
        0,
        "searches returned fewer than {K} of {NODES} nodes"
    );
}
