use super::super::super::distance::CpuDistance;
use super::*;
use crate::distance::DistanceMetric;

/// Creates a small test index with `n` vectors of dimension `dim`.
fn build_test_index(n: usize, dim: usize) -> NativeHnsw<CpuDistance> {
    let distance = CpuDistance::new(DistanceMetric::Euclidean);
    let hnsw = NativeHnsw::new(distance, 8, 32, n);
    for i in 0..n {
        // Reason: cast_precision_loss acceptable for test data generation.
        #[allow(clippy::cast_precision_loss)]
        let v: Vec<f32> = (0..dim).map(|d| (i * dim + d) as f32).collect();
        hnsw.insert(&v).unwrap();
    }
    hnsw
}

#[test]
fn reorder_skips_small_index() {
    let hnsw = build_test_index(50, 4);
    // Should be a no-op for <1000 vectors
    assert!(hnsw.reorder_for_locality().is_ok());
}

#[test]
fn reorder_preserves_search_results() {
    let hnsw = build_test_index(1200, 4);

    // Search before reorder
    let query = vec![1.0, 2.0, 3.0, 4.0];
    let before = hnsw.search(&query, 5, 64);
    let _before_ids: Vec<NodeId> = before.iter().map(|(id, _)| *id).collect();

    // Reorder
    hnsw.reorder_for_locality().unwrap();

    // Search after reorder — results should contain the same vectors
    // (IDs change but the distances should be identical)
    let after = hnsw.search(&query, 5, 64);
    assert_eq!(before.len(), after.len(), "Result count changed");

    // Distances should match (order may differ by tie-breaking)
    let mut before_dists: Vec<f32> = before.iter().map(|(_, d)| *d).collect();
    let mut after_dists: Vec<f32> = after.iter().map(|(_, d)| *d).collect();
    before_dists.sort_by(f32::total_cmp);
    after_dists.sort_by(f32::total_cmp);
    for (b, a) in before_dists.iter().zip(after_dists.iter()) {
        assert!(
            (b - a).abs() < 1e-5,
            "Distance mismatch: before={b}, after={a}"
        );
    }

    // Verify entry point is still valid
    let ep_id = hnsw.entry_point.load(Ordering::Acquire);
    assert_ne!(ep_id, NO_ENTRY_POINT, "Entry point lost after reorder");
    assert!(
        ep_id < hnsw.count.load(Ordering::Relaxed),
        "Entry point out of bounds"
    );

    // Verify all IDs in results are within bounds
    for (id, _) in &after {
        assert!(*id < hnsw.count.load(Ordering::Relaxed));
    }
}

#[test]
fn bfs_order_covers_all_nodes() {
    let hnsw = build_test_index(1200, 4);
    let count = hnsw.count.load(Ordering::Relaxed);
    let ep = hnsw.entry_point.load(Ordering::Acquire);
    assert_ne!(ep, NO_ENTRY_POINT, "entry_point must be set");
    let order = hnsw.compute_bfs_order(ep, count);

    assert_eq!(order.len(), count, "BFS order must cover all nodes");

    // Verify it's a valid permutation (each node appears exactly once)
    let mut seen = vec![false; count];
    for &id in &order {
        assert!(id < count, "BFS order contains out-of-bounds ID: {id}");
        assert!(!seen[id], "BFS order contains duplicate ID: {id}");
        seen[id] = true;
    }
}

#[test]
fn reverse_mapping_is_inverse() {
    let new_order = vec![3, 1, 4, 0, 2];
    let old_to_new = NativeHnsw::<CpuDistance>::build_reverse_mapping(&new_order, 5);
    // new_order[0] = 3  => old_to_new[3] = 0
    // new_order[1] = 1  => old_to_new[1] = 1
    // new_order[2] = 4  => old_to_new[4] = 2
    // new_order[3] = 0  => old_to_new[0] = 3
    // new_order[4] = 2  => old_to_new[2] = 4
    assert_eq!(old_to_new, vec![3, 1, 4, 0, 2]);
}
