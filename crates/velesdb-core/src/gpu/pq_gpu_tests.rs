use super::*;

#[test]
fn test_should_use_gpu_threshold() {
    // Below threshold: 100 * 16 * 8 = 12800 < 10M
    assert!(!should_use_gpu(100, 16, 8));

    // Above threshold: 10000 * 256 * 8 = 20_480_000 > 10M
    assert!(should_use_gpu(10000, 256, 8));

    // Exactly at threshold: should not trigger (strictly greater)
    assert!(!should_use_gpu(10_000_000 / (256 * 8), 256, 8));
}

#[test]
fn test_gpu_context_new_does_not_panic() {
    // PqGpuContext::new() either succeeds or returns None -- must not panic.
    // This validates the singleton delegation works regardless of whether
    // we are in an async runtime.
    let _ctx = PqGpuContext::new();
    // No assertion: GPU may not be available in CI. Absence of panic is the test.
}

#[test]
fn test_gpu_kmeans_assign_matches_cpu() {
    // Small dataset: 10 vectors, 3 centroids, dim=4
    let sub_vectors = vec![
        vec![1.0, 0.0, 0.0, 0.0],
        vec![0.9, 0.1, 0.0, 0.0],
        vec![0.0, 1.0, 0.0, 0.0],
        vec![0.1, 0.9, 0.0, 0.0],
        vec![0.0, 0.0, 1.0, 0.0],
        vec![0.0, 0.0, 0.9, 0.1],
        vec![1.0, 1.0, 0.0, 0.0],
        vec![0.0, 0.0, 0.0, 1.0],
        vec![0.5, 0.5, 0.0, 0.0],
        vec![0.0, 0.0, 0.5, 0.5],
    ];
    let centroids = vec![
        vec![1.0, 0.0, 0.0, 0.0],
        vec![0.0, 1.0, 0.0, 0.0],
        vec![0.0, 0.0, 1.0, 0.0],
    ];

    // CPU assignments (nearest centroid by L2)
    let cpu_assignments: Vec<usize> = sub_vectors
        .iter()
        .map(|v| {
            centroids
                .iter()
                .enumerate()
                .map(|(idx, c)| {
                    let dist: f32 = v.iter().zip(c.iter()).map(|(a, b)| (a - b) * (a - b)).sum();
                    (idx, dist)
                })
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(idx, _)| idx)
                .unwrap()
        })
        .collect();

    // GPU assignments (may return None if no GPU available or context init fails)
    if let Some(ctx) = PqGpuContext::new() {
        if let Some(gpu_assignments) = gpu_kmeans_assign(&ctx, &sub_vectors, &centroids, 4) {
            assert_eq!(
                gpu_assignments.len(),
                sub_vectors.len(),
                "GPU must return one assignment per vector"
            );
            assert_eq!(
                gpu_assignments, cpu_assignments,
                "GPU assignments must match CPU"
            );
        }
    }
    // If GPU is not available, test passes silently (fallback behavior)
}

#[test]
fn test_gpu_kmeans_assign_empty_input() {
    if let Some(ctx) = PqGpuContext::new() {
        assert!(gpu_kmeans_assign(&ctx, &[], &[vec![1.0]], 1).is_none());
        assert!(gpu_kmeans_assign(&ctx, &[vec![1.0]], &[], 1).is_none());
        assert!(gpu_kmeans_assign(&ctx, &[vec![1.0]], &[vec![1.0]], 0).is_none());
    }
}

#[test]
fn test_gpu_kmeans_assign_dimension_mismatch_returns_none() {
    // sub_vectors[0] has dim=3 but subspace_dim=4 -> must return None
    if let Some(ctx) = PqGpuContext::new() {
        let sub_vectors = vec![vec![1.0, 0.0, 0.0]]; // dim=3
        let centroids = vec![vec![1.0, 0.0, 0.0, 0.0]]; // dim=4
        assert!(
            gpu_kmeans_assign(&ctx, &sub_vectors, &centroids, 4).is_none(),
            "mismatched sub_vector dim must return None"
        );
    }
}

/// Regression guard: `PqGpuContext::new()` must return `Some` if and only if
/// `GpuAccelerator::is_available()` returns `true`. After consolidation
/// (Step 0.16), both go through the same singleton -- this test ensures
/// they stay in sync.
#[test]
fn test_pq_context_shares_global_device() {
    let gpu_available = GpuAccelerator::is_available();
    let pq_ctx = PqGpuContext::new();

    assert_eq!(
        pq_ctx.is_some(),
        gpu_available,
        "PqGpuContext availability must match GpuAccelerator::is_available()"
    );
}
