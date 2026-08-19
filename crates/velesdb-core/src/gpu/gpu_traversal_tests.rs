use super::*;
use proptest::prelude::*;

#[test]
fn test_should_traverse_gpu_threshold() {
    // Performance gate (num_vectors > 500_000) — dimension 128 is always safe.
    assert!(!should_traverse_gpu(0, 128));
    assert!(!should_traverse_gpu(100_000, 128));
    assert!(!should_traverse_gpu(500_000, 128));
    assert!(should_traverse_gpu(500_001, 128));
    assert!(should_traverse_gpu(1_000_000, 128));
}

#[test]
fn test_should_traverse_gpu_u32_offset_correctness_gate() {
    // 10M * 768 = 7_680_000_000 > u32::MAX (4_294_967_295) — must bail out to CPU.
    assert!(!should_traverse_gpu(10_000_000, 768));
    // 5M * 768 = 3_840_000_000 < u32::MAX — safe.
    assert!(should_traverse_gpu(5_000_000, 768));
    // Boundary: exactly u32::MAX offsets — safe.
    assert!(should_traverse_gpu((u32::MAX as usize) / 128, 128));
    // Overflow even after checked_mul — must bail out.
    assert!(!should_traverse_gpu(usize::MAX / 2, 4));
}

#[test]
fn test_gpu_traversal_context_new_no_panic() {
    // GpuTraversalContext::new() should not panic even without GPU
    let _ctx = GpuTraversalContext::new();
    // May return None if no GPU — that's fine
}

#[test]
fn test_search_empty_csr_returns_empty() {
    if let Some(ctx) = GpuTraversalContext::new() {
        let csr = CsrGraph {
            offsets: vec![0],
            neighbors: vec![],
            num_nodes: 0,
            max_degree: 0,
            total_edges: 0,
        };
        let result = ctx.search_layer0(
            &csr,
            &[],
            &[1.0, 0.0, 0.0],
            0,
            10,
            64,
            3,
            crate::distance::DistanceMetric::Cosine,
        );
        assert!(result.is_empty());
    }
}

#[test]
fn test_search_unsupported_metric_returns_empty() {
    if let Some(ctx) = GpuTraversalContext::new() {
        let csr = CsrGraph {
            offsets: vec![0, 1],
            neighbors: vec![0],
            num_nodes: 1,
            max_degree: 1,
            total_edges: 1,
        };
        // Hamming has no GPU shader
        let result = ctx.search_layer0(
            &csr,
            &[1.0, 0.0, 0.0],
            &[1.0, 0.0, 0.0],
            0,
            10,
            64,
            3,
            crate::distance::DistanceMetric::Hamming,
        );
        assert!(result.is_empty());
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(300))]

    /// `adaptive_gpu_iterations` output is always within [10, 20].
    #[test]
    fn prop_adaptive_iterations_in_range(ef in 0usize..100_000usize) {
        let iters = adaptive_gpu_iterations(ef);
        prop_assert!(
            (10..=20).contains(&iters),
            "iterations={iters} out of [10,20] for ef_search={ef}"
        );
    }

    /// `adaptive_gpu_iterations` is monotone non-increasing over its full domain.
    /// Generates strictly-ordered pairs to cover every step boundary, not just 64/65.
    #[test]
    fn prop_adaptive_iterations_monotone_nonincreasing(
        (ef_lo, ef_hi) in (0usize..99_999usize)
            .prop_flat_map(|lo| (lo + 1..100_000usize).prop_map(move |hi| (lo, hi))),
    ) {
        prop_assert!(
            adaptive_gpu_iterations(ef_lo) >= adaptive_gpu_iterations(ef_hi),
            "iters(ef={ef_lo}) must be >= iters(ef={ef_hi})"
        );
    }

    /// Performance gate: n ≤ 500_000 always returns false regardless of dimension.
    #[test]
    fn prop_should_traverse_gpu_small_n_always_false(
        n in 0usize..=500_000usize,
        dim in 1usize..=4096usize,
    ) {
        prop_assert!(
            !should_traverse_gpu(n, dim),
            "n={n} <= 500_000 must return false (perf gate), dim={dim}"
        );
    }

    /// Euclidean squared distance from a vector to itself is 0.
    #[test]
    fn prop_cpu_distance_euclidean_identity(
        v in proptest::collection::vec(0.01f32..=1.0f32, 1..=64usize),
    ) {
        let d = gpu_distance_cpu_fallback(&v, &v, crate::distance::DistanceMetric::Euclidean);
        prop_assert!(d.abs() < 1e-5, "Euclidean sq(v,v) must be 0, got {d}");
    }

    /// Cosine distance from a non-zero vector to itself is ≈0.
    #[test]
    fn prop_cpu_distance_cosine_identity(
        v in proptest::collection::vec(0.01f32..=1.0f32, 1..=64usize),
    ) {
        let d = gpu_distance_cpu_fallback(&v, &v, crate::distance::DistanceMetric::Cosine);
        prop_assert!(d.abs() < 1e-5, "Cosine dist(v,v) must be ~0, got {d}");
    }

    /// Euclidean squared distance is non-negative for any two same-length vectors.
    #[test]
    fn prop_cpu_distance_euclidean_nonneg(
        vw in (1usize..=32usize).prop_flat_map(|len| (
            proptest::collection::vec(-1.0f32..=1.0f32, len),
            proptest::collection::vec(-1.0f32..=1.0f32, len),
        )),
    ) {
        let (v, w) = vw;
        let d = gpu_distance_cpu_fallback(&v, &w, crate::distance::DistanceMetric::Euclidean);
        prop_assert!(d >= 0.0, "Euclidean sq must be non-negative, got {d}");
    }

    /// Euclidean squared distance is symmetric: d(a,b) == d(b,a).
    #[test]
    fn prop_cpu_distance_euclidean_symmetric(
        vw in (1usize..=32usize).prop_flat_map(|len| (
            proptest::collection::vec(-1.0f32..=1.0f32, len),
            proptest::collection::vec(-1.0f32..=1.0f32, len),
        )),
    ) {
        let (v, w) = vw;
        let d_ab = gpu_distance_cpu_fallback(&v, &w, crate::distance::DistanceMetric::Euclidean);
        let d_ba = gpu_distance_cpu_fallback(&w, &v, crate::distance::DistanceMetric::Euclidean);
        prop_assert!(
            (d_ab - d_ba).abs() < 1e-4,
            "Euclidean sq must be symmetric: d(a,b)={d_ab} vs d(b,a)={d_ba}"
        );
    }

    /// Cosine distance is in [0, 2] for two independently-generated non-zero vectors.
    #[test]
    fn prop_cpu_distance_cosine_in_range(
        vw in (1usize..=32usize).prop_flat_map(|len| (
            proptest::collection::vec(0.01f32..=1.0f32, len),
            proptest::collection::vec(0.01f32..=1.0f32, len),
        )),
    ) {
        let (v, w) = vw;
        let d = gpu_distance_cpu_fallback(&v, &w, crate::distance::DistanceMetric::Cosine);
        prop_assert!(
            ((-1e-5f32)..=(2.0_f32 + 1e-5)).contains(&d),
            "Cosine distance must be in [0,2], got {d}"
        );
    }

    /// DotProduct(v, v) = -‖v‖² ≤ 0 for any non-zero v.
    #[test]
    fn prop_cpu_distance_dot_product_self_nonpositive(
        v in proptest::collection::vec(0.01f32..=1.0f32, 1..=64usize),
    ) {
        let d = gpu_distance_cpu_fallback(&v, &v, crate::distance::DistanceMetric::DotProduct);
        prop_assert!(d <= 0.0, "DotProduct(v,v) = -‖v‖² must be <= 0, got {d}");
    }

    /// DotProduct is symmetric since the dot product is commutative.
    #[test]
    fn prop_cpu_distance_dot_product_symmetric(
        vw in (1usize..=32usize).prop_flat_map(|len| (
            proptest::collection::vec(-1.0f32..=1.0f32, len),
            proptest::collection::vec(-1.0f32..=1.0f32, len),
        )),
    ) {
        let (v, w) = vw;
        let d_ab = gpu_distance_cpu_fallback(&v, &w, crate::distance::DistanceMetric::DotProduct);
        let d_ba = gpu_distance_cpu_fallback(&w, &v, crate::distance::DistanceMetric::DotProduct);
        prop_assert!(
            (d_ab - d_ba).abs() < 1e-4,
            "DotProduct must be symmetric: d(a,b)={d_ab} vs d(b,a)={d_ba}"
        );
    }

    /// Hamming and Jaccard return f32::MAX — no GPU shader for binary metrics.
    #[test]
    fn prop_cpu_distance_unsupported_metrics_return_max(
        v in proptest::collection::vec(0.1f32..=1.0f32, 1..=16usize),
    ) {
        prop_assert!(
            gpu_distance_cpu_fallback(&v, &v, crate::distance::DistanceMetric::Hamming)
                .to_bits() == f32::MAX.to_bits(),
            "Hamming must return f32::MAX (no GPU shader)"
        );
        prop_assert!(
            gpu_distance_cpu_fallback(&v, &v, crate::distance::DistanceMetric::Jaccard)
                .to_bits() == f32::MAX.to_bits(),
            "Jaccard must return f32::MAX (no GPU shader)"
        );
    }
}
