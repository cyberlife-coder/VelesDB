//! Extended CSR tests for validate(), density(), avg_degree(), Display,
//! high-degree graphs, isolated nodes, large-scale, and concurrent access.

use crate::gpu::gpu_csr::{CsrGraph, CsrCache};
use crate::index::hnsw::native::layer::{Layer, NodeId};

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
                let csr = c.get_or_rebuild(&l, 100);
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
fn test_csr_cache_invalidate_rebuild_cycle() {
    let layer = Layer::new(10);
    for i in 0..10 {
        layer.set_neighbors(i, vec![(i + 1) % 10]);
    }
    let cache = CsrCache::new();
    for round in 0..5_u64 {
        let csr = cache.get_or_rebuild(&layer, 10);
        assert_eq!(csr.num_nodes, 10);
        assert_eq!(cache.version(), round + 1);
        cache.invalidate();
    }
}
