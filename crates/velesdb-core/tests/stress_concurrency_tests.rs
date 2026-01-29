//! Stress tests for concurrent Collection operations.
//!
//! # Design Decision
//!
//! Uses **finite operations** per thread instead of time-based loops to avoid
//! writer starvation. See SOUNDNESS.md "Writer Starvation Prevention" section.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Instant;
use tempfile::tempdir;
use velesdb_core::collection::graph::{ConcurrentEdgeStore, GraphEdge};
use velesdb_core::distance::DistanceMetric;
use velesdb_core::point::Point;
use velesdb_core::Collection;

#[allow(clippy::cast_precision_loss)]
fn generate_vector(dimension: usize, seed: u64) -> Vec<f32> {
    let mut v = Vec::with_capacity(dimension);
    let mut x = seed;
    for _ in 0..dimension {
        x = x.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        v.push((x as f32 / u64::MAX as f32) * 2.0 - 1.0);
    }
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in &mut v {
            *x /= norm;
        }
    }
    v
}

/// Smoke test: 5 readers + 5 writers × 20 ops
#[test]
fn test_stress_smoke_10_threads() {
    run_collection_stress(5, 5, 20, 64, 50);
}

/// Medium stress: 10+10 threads × 50 ops
#[test]
fn test_stress_medium_20_threads() {
    run_collection_stress(10, 10, 50, 64, 100);
}

/// Heavy stress: 25+25 threads × 100 ops (ignored for CI)
#[test]
#[ignore = "Heavy stress test, run manually"]
fn test_stress_50_threads() {
    run_collection_stress(25, 25, 100, 128, 500);
}

#[allow(clippy::cast_precision_loss)]
fn run_collection_stress(
    num_readers: usize,
    num_writers: usize,
    ops_per_thread: usize,
    dimension: usize,
    initial_points: usize,
) {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("stress");

    let collection =
        Arc::new(Collection::create(path, dimension, DistanceMetric::Cosine).expect("create"));

    // Seed
    let initial: Vec<Point> = (0..initial_points as u64)
        .map(|i| Point::without_payload(i, generate_vector(dimension, i)))
        .collect();
    collection.upsert(initial).expect("seed");

    let next_id = Arc::new(AtomicU64::new(initial_points as u64));
    let searches = Arc::new(AtomicU64::new(0));
    let writes = Arc::new(AtomicU64::new(0));

    let mut handles = Vec::new();
    let start = Instant::now();

    // Readers (finite ops)
    for t in 0..num_readers {
        let coll = Arc::clone(&collection);
        let cnt = Arc::clone(&searches);
        handles.push(thread::spawn(move || {
            for i in 0..ops_per_thread {
                let query = generate_vector(dimension, (t * 1000 + i) as u64);
                if coll.search(&query, 10).is_ok() {
                    cnt.fetch_add(1, Ordering::Relaxed);
                }
            }
        }));
    }

    // Writers (finite ops)
    for t in 0..num_writers {
        let coll = Arc::clone(&collection);
        let nid = Arc::clone(&next_id);
        let cnt = Arc::clone(&writes);
        let max = initial_points as u64;
        handles.push(thread::spawn(move || {
            for i in 0..ops_per_thread {
                let id = if i % 2 == 0 {
                    ((t * 10000 + i) as u64) % max
                } else {
                    nid.fetch_add(1, Ordering::Relaxed)
                };
                let pt = Point::without_payload(id, generate_vector(dimension, id));
                if coll.upsert(vec![pt]).is_ok() {
                    cnt.fetch_add(1, Ordering::Relaxed);
                }
            }
        }));
    }

    for h in handles {
        h.join().expect("thread join");
    }

    let elapsed = start.elapsed();
    let s = searches.load(Ordering::Relaxed);
    let w = writes.load(Ordering::Relaxed);
    println!(
        "Collection stress: {:.2}s, {} searches, {} writes ({:.0} ops/sec)",
        elapsed.as_secs_f64(),
        s,
        w,
        (s + w) as f64 / elapsed.as_secs_f64()
    );

    assert!(collection
        .search(&generate_vector(dimension, 999), 5)
        .is_ok());
    assert!(collection.flush().is_ok());
}

/// Graph stress: 10+10 threads × 100 ops on `ConcurrentEdgeStore`
#[test]
fn test_graph_concurrent_stress() {
    let store = Arc::new(ConcurrentEdgeStore::new());
    let next_id = Arc::new(AtomicU64::new(1));
    let ops = 100;

    let mut handles = Vec::new();

    // Writers
    for t in 0..10 {
        let s = Arc::clone(&store);
        let eid = Arc::clone(&next_id);
        handles.push(thread::spawn(move || {
            for i in 0..ops {
                let id = eid.fetch_add(1, Ordering::Relaxed);
                #[allow(clippy::cast_sign_loss)]
                let src = (t as u64 * 1000 + i as u64) % 1000;
                let tgt = (src + 1) % 1000;
                if let Ok(e) = GraphEdge::new(id, src, tgt, "LINK") {
                    let _ = s.add_edge(e);
                }
                if i % 10 == 0 {
                    s.remove_edge(id.saturating_sub(5));
                }
            }
        }));
    }

    // Readers
    for t in 0..10 {
        let s = Arc::clone(&store);
        handles.push(thread::spawn(move || {
            for i in 0..ops {
                #[allow(clippy::cast_sign_loss)]
                let n = ((t * 100 + i) % 1000) as u64;
                let _ = s.get_outgoing(n);
                let _ = s.get_incoming(n);
                let _ = s.traverse_bfs(n, 2);
            }
        }));
    }

    for h in handles {
        h.join().expect("graph thread");
    }

    println!("Graph stress: {} edges", store.edge_count());
}
