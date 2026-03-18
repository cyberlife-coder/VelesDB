use crate::internal_bench;
use crate::simd_native::{cosine_similarity_native, DistanceEngine};

fn sample_vectors(dim: usize) -> (Vec<f32>, Vec<f32>) {
    let a = (0..dim)
        .map(|i| {
            let idx = f32::from(u16::try_from(i).expect("test dimensions fit in u16"));
            (idx * 0.13).sin()
        })
        .collect();
    let b = (0..dim)
        .map(|i| {
            let idx = f32::from(u16::try_from(i).expect("test dimensions fit in u16"));
            (idx * 0.17 + 1.0).cos()
        })
        .collect();
    (a, b)
}

#[test]
fn test_internal_bench_cosine_paths_match_public_dispatch() {
    for dim in [0_usize, 1, 7, 8, 63, 64, 127, 128, 767, 768, 769, 1536] {
        let (a, b) = sample_vectors(dim);
        let dispatch = cosine_similarity_native(&a, &b);
        let scalar = internal_bench::cosine_scalar(&a, &b);
        let resolved = internal_bench::cosine_resolved(&a, &b);
        assert!((dispatch - scalar).abs() <= 1e-5, "dim={dim}");
        assert!((dispatch - resolved).abs() <= 1e-5, "dim={dim}");
    }
}

#[test]
fn test_internal_bench_distance_engine_matches_public_engine() {
    for dim in [64_usize, 768, 1536] {
        let (a, b) = sample_vectors(dim);
        let public_engine = DistanceEngine::new(dim);
        let internal = internal_bench::cosine_resolved(&a, &b);
        let public = public_engine.cosine_similarity(&a, &b);
        assert!((internal - public).abs() <= 1e-5, "dim={dim}");
    }
}
