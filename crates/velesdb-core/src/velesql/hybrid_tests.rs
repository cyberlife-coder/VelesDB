//! Tests for `hybrid` module - Hybrid search fusion strategies.

use super::hybrid::*;

fn make_results(ids_scores: &[(u64, f32)]) -> Vec<ScoredResult> {
    ids_scores
        .iter()
        .map(|(id, score)| ScoredResult::new(*id, *score))
        .collect()
}

#[test]
fn test_rrf_basic() {
    let vector = make_results(&[(1, 0.9), (2, 0.8), (3, 0.7)]);
    let graph = make_results(&[(2, 1.0), (1, 0.5), (4, 0.3)]);

    let fused = fuse_rrf(&vector, &graph, &RrfConfig::default(), 10);

    assert!(fused[0].id == 1 || fused[0].id == 2);
    assert!(fused[1].id == 1 || fused[1].id == 2);
    assert_ne!(fused[0].id, fused[1].id);
    assert_eq!(fused.len(), 4);
}

#[test]
fn test_rrf_k_parameter() {
    let vector = make_results(&[(1, 0.9)]);
    let graph = make_results(&[(1, 1.0)]);

    let fused_k60 = fuse_rrf(&vector, &graph, &RrfConfig::with_k(60), 10);
    let fused_k1 = fuse_rrf(&vector, &graph, &RrfConfig::with_k(1), 10);

    assert!(fused_k1[0].score > fused_k60[0].score);
}

#[test]
fn test_weighted_fusion() {
    let vector = make_results(&[(1, 1.0), (2, 0.5)]);
    let graph = make_results(&[(2, 1.0), (1, 0.5)]);

    let config = WeightedConfig::new(0.5, 0.5);
    let fused = fuse_weighted(&vector, &graph, &config, 10);

    assert!((fused[0].score - fused[1].score).abs() < 0.1);
}

#[test]
fn test_weighted_vector_preference() {
    let vector = make_results(&[(1, 1.0), (2, 0.0)]);
    let graph = make_results(&[(2, 1.0), (1, 0.0)]);

    let config = WeightedConfig::new(0.9, 0.1);
    let fused = fuse_weighted(&vector, &graph, &config, 10);

    assert_eq!(fused[0].id, 1);
}

#[test]
fn test_maximum_fusion() {
    let vector = make_results(&[(1, 0.9), (2, 0.3)]);
    let graph = make_results(&[(2, 0.8), (3, 0.7)]);

    let fused = fuse_maximum(&vector, &graph, 10);

    // After normalization, ID 1 and ID 2 both get score 1.0
    // (ID 1 is max in vector, ID 2 is max in graph)
    // So either can be first due to HashMap iteration order
    assert!(fused[0].id == 1 || fused[0].id == 2);
    assert_eq!(fused.len(), 3);
}

#[test]
fn test_intersect_results() {
    let vector = make_results(&[(1, 0.9), (2, 0.8), (3, 0.7)]);
    let graph = make_results(&[(2, 1.0), (3, 0.5), (4, 0.3)]);

    let (v_filtered, g_filtered) = intersect_results(&vector, &graph);

    assert_eq!(v_filtered.len(), 2);
    assert_eq!(g_filtered.len(), 2);
    assert!(v_filtered.iter().all(|r| r.id == 2 || r.id == 3));
}

#[test]
fn test_empty_results() {
    let vector = make_results(&[(1, 0.9)]);
    let empty: Vec<ScoredResult> = vec![];

    let fused = fuse_rrf(&vector, &empty, &RrfConfig::default(), 10);
    assert_eq!(fused.len(), 1);
    assert_eq!(fused[0].id, 1);
}

#[test]
fn test_limit_respected() {
    let vector = make_results(&[(1, 0.9), (2, 0.8), (3, 0.7), (4, 0.6), (5, 0.5)]);
    let graph = make_results(&[(6, 1.0), (7, 0.5)]);

    let fused = fuse_rrf(&vector, &graph, &RrfConfig::default(), 3);
    assert_eq!(fused.len(), 3);
}

#[test]
fn test_normalize_scores() {
    let results = make_results(&[(1, 100.0), (2, 50.0), (3, 0.0)]);
    let normalized = normalize_scores(&results);

    assert!((normalized[0].score - 1.0).abs() < 0.001);
    assert!((normalized[1].score - 0.5).abs() < 0.001);
    assert!((normalized[2].score - 0.0).abs() < 0.001);
}
