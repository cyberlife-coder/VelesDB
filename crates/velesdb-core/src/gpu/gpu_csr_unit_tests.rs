use super::*;
use crate::index::hnsw::native::{
    hnsw_holds_lock, hnsw_record_lock_acquire, hnsw_record_lock_release, HnswLockRank, NodeId,
};

/// Runs `f` with the layers rank recorded as held — the caller contract
/// of [`CsrCache::get_or_rebuild`]. Abstracts away the record/release
/// dance so tests read linearly and do not accidentally break the
/// contract by forgetting the release.
fn with_layers_rank<R>(f: impl FnOnce() -> R) -> R {
    hnsw_record_lock_acquire(HnswLockRank::Layers);
    debug_assert!(hnsw_holds_lock(HnswLockRank::Layers));
    let result = f();
    hnsw_record_lock_release(HnswLockRank::Layers);
    result
}

#[test]
fn test_csr_from_empty_layer() {
    let layer = Layer::new(0);
    let csr = CsrGraph::from_layer(&layer, 0);
    assert!(csr.is_empty());
    assert_eq!(csr.offsets, vec![0]);
    assert!(csr.neighbors.is_empty());
    assert_eq!(csr.max_degree, 0);
    assert_eq!(csr.total_edges, 0);
}

#[test]
fn test_csr_from_simple_layer() {
    let layer = Layer::new(4);
    layer.set_neighbors(0, vec![1, 2]);
    layer.set_neighbors(1, vec![0, 3]);
    layer.set_neighbors(2, vec![0, 1, 3]);
    layer.set_neighbors(3, vec![1, 2]);

    let csr = CsrGraph::from_layer(&layer, 4);
    assert_eq!(csr.num_nodes, 4);
    assert_eq!(csr.offsets, vec![0, 2, 4, 7, 9]);
    assert_eq!(csr.neighbors, vec![1, 2, 0, 3, 0, 1, 3, 1, 2]);
    assert_eq!(csr.max_degree, 3);
    assert_eq!(csr.total_edges, 9);
}

#[test]
fn test_csr_neighbor_lookup() {
    let layer = Layer::new(3);
    layer.set_neighbors(0, vec![1, 2]);
    layer.set_neighbors(1, vec![]);
    layer.set_neighbors(2, vec![0]);

    let csr = CsrGraph::from_layer(&layer, 3);

    // Node 0: neighbors at offsets[0]..offsets[1] = 0..2
    assert_eq!(
        &csr.neighbors[csr.offsets[0] as usize..csr.offsets[1] as usize],
        &[1, 2]
    );
    // Node 1: neighbors at offsets[1]..offsets[2] = 2..2 (empty)
    assert_eq!(
        &csr.neighbors[csr.offsets[1] as usize..csr.offsets[2] as usize],
        &[] as &[u32]
    );
    // Node 2: neighbors at offsets[2]..offsets[3] = 2..3
    assert_eq!(
        &csr.neighbors[csr.offsets[2] as usize..csr.offsets[3] as usize],
        &[0]
    );
}

#[test]
fn test_csr_cache_dirty_flag() {
    let cache = CsrCache::new();
    assert_eq!(cache.version(), 0);

    let layer = Layer::new(2);
    layer.set_neighbors(0, vec![1]);
    layer.set_neighbors(1, vec![0]);

    // First build — tests model the production caller contract
    // (`with_layers_read → get_or_rebuild`) via `with_layers_rank`.
    let csr = with_layers_rank(|| cache.get_or_rebuild(&layer, 2));
    assert_eq!(csr.num_nodes, 2);
    assert_eq!(cache.version(), 1);

    // Should return cached (not rebuild)
    let csr2 = with_layers_rank(|| cache.get_or_rebuild(&layer, 2));
    assert_eq!(csr2.num_nodes, 2);
    assert_eq!(cache.version(), 1); // Same version

    // Invalidate and rebuild
    cache.invalidate();
    let csr3 = with_layers_rank(|| cache.get_or_rebuild(&layer, 2));
    assert_eq!(csr3.num_nodes, 2);
    assert_eq!(cache.version(), 2); // Incremented
}

#[test]
fn test_csr_byte_sizes() {
    let layer = Layer::new(100);
    for i in 0..100 {
        let neighbors: Vec<NodeId> = (0..16).map(|j| (i + j + 1) % 100).collect();
        layer.set_neighbors(i, neighbors);
    }

    let csr = CsrGraph::from_layer(&layer, 100);
    assert_eq!(csr.offsets_byte_size(), 101 * 4); // (N+1) * sizeof(u32)
    assert_eq!(csr.neighbors_byte_size(), 1600 * 4); // 100 * 16 * sizeof(u32)
    assert_eq!(csr.total_gpu_bytes(), 101 * 4 + 1600 * 4);
}

#[test]
fn test_csr_partial_capacity() {
    // Layer pre-allocated for 100 but only 5 nodes are active
    let layer = Layer::new(100);
    layer.set_neighbors(0, vec![1, 2]);
    layer.set_neighbors(1, vec![0]);

    let csr = CsrGraph::from_layer(&layer, 5);
    assert_eq!(csr.num_nodes, 5);
    // Nodes 2..4 should have zero neighbors
    assert_eq!(csr.offsets[2], csr.offsets[3]);
    assert_eq!(csr.offsets[3], csr.offsets[4]);
    assert_eq!(csr.offsets[4], csr.offsets[5]);
}

#[test]
fn test_clean_snapshot_returns_none_when_dirty() {
    let cache = CsrCache::new();
    assert!(cache.clean_snapshot().is_none()); // Starts dirty

    let layer = Layer::new(1);
    with_layers_rank(|| cache.get_or_rebuild(&layer, 1)); // Build it
    assert!(cache.clean_snapshot().is_some()); // Now clean

    cache.invalidate();
    assert!(cache.clean_snapshot().is_none()); // Dirty again
}
