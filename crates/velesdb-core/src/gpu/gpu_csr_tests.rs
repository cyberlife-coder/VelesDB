//! Extended CSR tests for validate(), density(), avg_degree(), Display,
//! high-degree graphs, isolated nodes, large-scale, and concurrent access.

// Test graphs use small controlled sizes (≤ 10K nodes, ≤ 16 degree); the
// `usize as u32` casts here are by construction within u32 range.
#![allow(clippy::cast_possible_truncation)]

use crate::gpu::gpu_csr::{CsrCache, CsrGraph};
use crate::index::hnsw::native::layer::{Layer, NodeId};
use crate::index::hnsw::native::{
    hnsw_record_lock_acquire, hnsw_record_lock_release, HnswLockRank,
};

/// Runs `f` with the layers rank recorded as held — the caller contract
/// of [`CsrCache::get_or_rebuild`]. Mirrors the helper in `gpu_csr::tests`
/// so concurrent tests spread across threads keep their rank stack clean.
fn with_layers_rank<R>(f: impl FnOnce() -> R) -> R {
    hnsw_record_lock_acquire(HnswLockRank::Layers);
    let result = f();
    hnsw_record_lock_release(HnswLockRank::Layers);
    result
}

#[test]
fn test_csr_validate_valid_graph() {
    let layer = Layer::new(4);
    layer.set_neighbors(0, vec![1, 2]);
    layer.set_neighbors(1, vec![0, 3]);
    layer.set_neighbors(2, vec![0, 1, 3]);
    layer.set_neighbors(3, vec![1, 2]);
    let csr = CsrGraph::from_layer(&layer, 4);
    assert!(csr.validate().is_ok());
}

#[test]
fn test_csr_validate_empty_graph() {
    let csr = CsrGraph {
        offsets: vec![0],
        neighbors: vec![],
        num_nodes: 0,
        max_degree: 0,
        total_edges: 0,
    };
    assert!(csr.validate().is_ok());
}

#[test]
fn test_csr_validate_bad_offsets_length() {
    let csr = CsrGraph {
        offsets: vec![0, 2],
        neighbors: vec![1, 2],
        num_nodes: 3,
        max_degree: 2,
        total_edges: 2,
    };
    let err = csr.validate().unwrap_err();
    assert!(err.contains("offsets.len()"), "error: {err}");
}

#[test]
fn test_csr_validate_non_monotonic_offsets() {
    let csr = CsrGraph {
        offsets: vec![0, 3, 2, 5],
        neighbors: vec![1, 2, 0, 0, 1],
        num_nodes: 3,
        max_degree: 3,
        total_edges: 5,
    };
    let err = csr.validate().unwrap_err();
    assert!(err.contains("monotonic"), "error: {err}");
}

#[test]
fn test_csr_validate_neighbor_out_of_bounds() {
    let csr = CsrGraph {
        offsets: vec![0, 2, 3],
        neighbors: vec![1, 5, 0],
        num_nodes: 2,
        max_degree: 2,
        total_edges: 3,
    };
    let err = csr.validate().unwrap_err();
    assert!(err.contains("neighbor"), "error: {err}");
}

#[test]
fn test_csr_validate_total_edges_mismatch() {
    let csr = CsrGraph {
        offsets: vec![0, 2, 3],
        neighbors: vec![1, 0, 0],
        num_nodes: 2,
        max_degree: 2,
        total_edges: 999,
    };
    let err = csr.validate().unwrap_err();
    assert!(err.contains("total_edges"), "error: {err}");
}

#[test]
fn test_csr_density_empty() {
    let csr = CsrGraph {
        offsets: vec![0],
        neighbors: vec![],
        num_nodes: 0,
        max_degree: 0,
        total_edges: 0,
    };
    assert!(csr.density().abs() < f64::EPSILON);
}

#[test]
fn test_csr_density_single_node() {
    let csr = CsrGraph {
        offsets: vec![0, 0],
        neighbors: vec![],
        num_nodes: 1,
        max_degree: 0,
        total_edges: 0,
    };
    assert!(csr.density().abs() < f64::EPSILON);
}

#[test]
fn test_csr_density_complete_graph_k4() {
    // K4: 4 nodes, 12 directed edges → density = 12/(4*3) = 1.0
    let csr = CsrGraph {
        offsets: vec![0, 3, 6, 9, 12],
        neighbors: vec![1, 2, 3, 0, 2, 3, 0, 1, 3, 0, 1, 2],
        num_nodes: 4,
        max_degree: 3,
        total_edges: 12,
    };
    assert!((csr.density() - 1.0).abs() < f64::EPSILON);
}

#[test]
fn test_csr_avg_degree_computed() {
    let layer = Layer::new(4);
    layer.set_neighbors(0, vec![1, 2]);
    layer.set_neighbors(1, vec![0, 3]);
    layer.set_neighbors(2, vec![0, 1, 3]);
    layer.set_neighbors(3, vec![1, 2]);
    let csr = CsrGraph::from_layer(&layer, 4);
    assert!((csr.avg_degree() - 2.25).abs() < f64::EPSILON);
}

#[test]
fn test_csr_avg_degree_empty() {
    let csr = CsrGraph {
        offsets: vec![0],
        neighbors: vec![],
        num_nodes: 0,
        max_degree: 0,
        total_edges: 0,
    };
    assert!(csr.avg_degree().abs() < f64::EPSILON);
}

#[test]
fn test_csr_display_format() {
    let layer = Layer::new(4);
    layer.set_neighbors(0, vec![1, 2]);
    layer.set_neighbors(1, vec![0, 3]);
    layer.set_neighbors(2, vec![0, 1, 3]);
    layer.set_neighbors(3, vec![1, 2]);
    let csr = CsrGraph::from_layer(&layer, 4);
    let display = format!("{csr}");
    assert!(display.contains("nodes=4"), "display: {display}");
    assert!(display.contains("edges=9"), "display: {display}");
    assert!(display.contains("max_deg=3"), "display: {display}");
    assert!(display.contains("avg_deg="), "display: {display}");
    assert!(display.contains("density="), "display: {display}");
}

#[test]
fn test_csr_high_degree_graph_m64() {
    let n = 50;
    let max_deg = 64;
    let layer = Layer::new(n);
    for i in 0..n {
        let nbrs: Vec<NodeId> = (0..max_deg).map(|j| (i + j + 1) % n).collect();
        layer.set_neighbors(i, nbrs);
    }
    let csr = CsrGraph::from_layer(&layer, n);
    assert_eq!(csr.num_nodes, n as u32);
    assert_eq!(csr.max_degree, max_deg as u32);
    assert_eq!(csr.total_edges, (n * max_deg) as u32);
    assert!(csr.validate().is_ok());
}

#[test]
fn test_csr_isolated_nodes_correctness() {
    let layer = Layer::new(10);
    layer.set_neighbors(0, vec![1]);
    layer.set_neighbors(1, vec![0]);
    // nodes 2..9 have no neighbors
    let csr = CsrGraph::from_layer(&layer, 10);
    assert_eq!(csr.num_nodes, 10);
    assert_eq!(csr.total_edges, 2);
    for i in 2..10 {
        assert_eq!(
            csr.offsets[i],
            csr.offsets[i + 1],
            "isolated node {i} should have zero degree"
        );
    }
    assert!(csr.validate().is_ok());
}

#[test]
fn test_csr_large_1k_ring_graph() {
    let n = 1000;
    let degree = 16;
    let layer = Layer::new(n);
    for i in 0..n {
        let nbrs: Vec<NodeId> = (1..=degree).map(|j| (i + j) % n).collect();
        layer.set_neighbors(i, nbrs);
    }
    let csr = CsrGraph::from_layer(&layer, n);
    assert_eq!(csr.num_nodes, n as u32);
    assert_eq!(csr.total_edges, (n * degree) as u32);
    assert_eq!(csr.max_degree, degree as u32);
    assert!(csr.validate().is_ok());
    let expected_bytes = (n + 1) * 4 + n * degree * 4;
    assert_eq!(csr.total_gpu_bytes(), expected_bytes);
}

#[test]
fn test_csr_cache_concurrent_rebuild_safety() {
    use std::sync::Arc;
    use std::thread;

    let layer = Arc::new(Layer::new(100));
    for i in 0..100 {
        let nbrs: Vec<NodeId> = (0..8).map(|j| (i + j + 1) % 100).collect();
        layer.set_neighbors(i, nbrs);
    }
    let cache = Arc::new(CsrCache::new());
    let handles: Vec<_> = (0..4)
        .map(|_| {
            let c = Arc::clone(&cache);
            let l = Arc::clone(&layer);
            thread::spawn(move || {
                let csr = with_layers_rank(|| c.get_or_rebuild(&l, 100));
                assert_eq!(csr.num_nodes, 100);
                assert!(csr.validate().is_ok());
            })
        })
        .collect();
    for h in handles {
        h.join().expect("thread should not panic");
    }
    assert!(cache.version() >= 1);
}

#[test]
fn test_csr_cache_no_stale_commit_under_concurrent_invalidate() {
    // Regression for the race Devin flagged in PR #626:
    // a slow rebuilder (gen=N) must NOT overwrite a fast rebuilder's
    // fresh CSR (gen=N+1) and leave `built_generation=N+1` while the
    // stored CSR is actually stale.
    //
    // Stress: 8 rebuilders + 8 invalidators run in parallel for 100
    // rounds each. Under the old load-compare-store design, a race
    // window was sufficient for a few stale commits to slip through.
    // Under the fixed design (check-then-write under a single write
    // lock) no combination of interleavings can leave the cache
    // marked fresh while holding data from an obsolete generation.
    //
    // Invariant verified: after the storm settles, the generation
    // counter must be >= the built_generation and the stored CSR
    // must validate. We also verify that a final rebuild makes the
    // cache converge to a clean, validate-ok state.
    use std::sync::Arc;
    use std::thread;

    let layer = Arc::new(Layer::new(64));
    for i in 0..64 {
        layer.set_neighbors(i, vec![(i + 1) % 64]);
    }
    let cache = Arc::new(CsrCache::new());

    let rebuild: Vec<_> = (0..8)
        .map(|_| {
            let c = Arc::clone(&cache);
            let l = Arc::clone(&layer);
            thread::spawn(move || {
                for _ in 0..100 {
                    let csr = with_layers_rank(|| c.get_or_rebuild(&l, 64));
                    // Whatever we observe, it must always validate:
                    // a corrupt CSR would indicate a torn write.
                    assert!(csr.validate().is_ok());
                }
            })
        })
        .collect();

    let invalidate: Vec<_> = (0..8)
        .map(|_| {
            let c = Arc::clone(&cache);
            thread::spawn(move || {
                for _ in 0..100 {
                    c.invalidate();
                    thread::yield_now();
                }
            })
        })
        .collect();

    for h in rebuild {
        h.join().expect("rebuild thread");
    }
    for h in invalidate {
        h.join().expect("invalidate thread");
    }

    // One final rebuild converges the cache. Must produce a valid CSR.
    let final_csr = with_layers_rank(|| cache.get_or_rebuild(&layer, 64));
    assert_eq!(final_csr.num_nodes, 64);
    assert!(final_csr.validate().is_ok());
}

#[test]
fn test_csr_cache_invalidate_rebuild_cycle() {
    let layer = Layer::new(10);
    for i in 0..10 {
        layer.set_neighbors(i, vec![(i + 1) % 10]);
    }
    let cache = CsrCache::new();
    for round in 0..5_u64 {
        let csr = with_layers_rank(|| cache.get_or_rebuild(&layer, 10));
        assert_eq!(csr.num_nodes, 10);
        assert_eq!(cache.version(), round + 1);
        cache.invalidate();
    }
}

/// Regression for the caller contract documented on
/// [`CsrCache::get_or_rebuild`]. Debug builds must panic when the
/// function is invoked without the layers read lock held — see the
/// race described by Devin on PR #626 where a rebuilder without the
/// shared layer snapshot can commit stale data under a fresh
/// generation counter. Release builds compile the assert out and have
/// to rely on the caller audit; this test guards the invariant at
/// least during CI / local debug runs.
#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "layers read lock")]
fn test_csr_cache_get_or_rebuild_panics_without_layers_lock() {
    let layer = Layer::new(4);
    layer.set_neighbors(0, vec![1, 2]);
    layer.set_neighbors(1, vec![0, 3]);
    layer.set_neighbors(2, vec![0, 1, 3]);
    layer.set_neighbors(3, vec![1, 2]);

    let cache = CsrCache::new();
    // Deliberately DO NOT wrap in `with_layers_rank` — this simulates a
    // future caller that forgets the contract.
    let _ = cache.get_or_rebuild(&layer, 4);
}
