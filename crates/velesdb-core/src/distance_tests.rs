//! Tests for `distance` module

use super::distance::*;

#[test]
fn test_cosine_similarity() {
    let a = vec![1.0, 0.0, 0.0];
    let b = vec![1.0, 0.0, 0.0];
    let similarity = DistanceMetric::Cosine.calculate(&a, &b);
    assert!((similarity - 1.0).abs() < 1e-6);

    let c = vec![0.0, 1.0, 0.0];
    let similarity = DistanceMetric::Cosine.calculate(&a, &c);
    assert!(similarity.abs() < 1e-6);
}

#[test]
fn test_euclidean_distance() {
    let a = vec![0.0, 0.0, 0.0];
    let b = vec![3.0, 4.0, 0.0];
    let distance = DistanceMetric::Euclidean.calculate(&a, &b);
    assert!((distance - 5.0).abs() < 1e-6);
}

#[test]
fn test_dot_product() {
    let a = vec![1.0, 2.0, 3.0];
    let b = vec![4.0, 5.0, 6.0];
    let product = DistanceMetric::DotProduct.calculate(&a, &b);
    assert!((product - 32.0).abs() < 1e-6);
}

#[test]
fn test_higher_is_better() {
    // Cosine: higher similarity = more similar
    assert!(DistanceMetric::Cosine.higher_is_better());

    // DotProduct: higher product = more similar
    assert!(DistanceMetric::DotProduct.higher_is_better());

    // Euclidean: lower distance = more similar
    assert!(!DistanceMetric::Euclidean.higher_is_better());
}

#[test]
fn test_metric_serialization() {
    // Test that metrics can be serialized/deserialized
    let metric = DistanceMetric::Cosine;
    let json = serde_json::to_string(&metric).unwrap();
    let deserialized: DistanceMetric = serde_json::from_str(&json).unwrap();
    assert_eq!(metric, deserialized);

    let metric = DistanceMetric::Euclidean;
    let json = serde_json::to_string(&metric).unwrap();
    let deserialized: DistanceMetric = serde_json::from_str(&json).unwrap();
    assert_eq!(metric, deserialized);

    let metric = DistanceMetric::DotProduct;
    let json = serde_json::to_string(&metric).unwrap();
    let deserialized: DistanceMetric = serde_json::from_str(&json).unwrap();
    assert_eq!(metric, deserialized);

    let metric = DistanceMetric::Hamming;
    let json = serde_json::to_string(&metric).unwrap();
    let deserialized: DistanceMetric = serde_json::from_str(&json).unwrap();
    assert_eq!(metric, deserialized);

    let metric = DistanceMetric::Jaccard;
    let json = serde_json::to_string(&metric).unwrap();
    let deserialized: DistanceMetric = serde_json::from_str(&json).unwrap();
    assert_eq!(metric, deserialized);
}

// =========================================================================
// TDD Tests for Hamming Distance (WIS-33)
// =========================================================================

#[test]
fn test_hamming_distance_identical() {
    // Identical binary vectors should have distance 0
    let a = vec![1.0, 0.0, 1.0, 0.0];
    let b = vec![1.0, 0.0, 1.0, 0.0];
    let distance = DistanceMetric::Hamming.calculate(&a, &b);
    assert!(
        (distance - 0.0).abs() < 1e-6,
        "Identical vectors: distance = 0"
    );
}

#[test]
fn test_hamming_distance_completely_different() {
    // Completely different binary vectors
    let a = vec![1.0, 1.0, 1.0, 1.0];
    let b = vec![0.0, 0.0, 0.0, 0.0];
    let distance = DistanceMetric::Hamming.calculate(&a, &b);
    assert!(
        (distance - 4.0).abs() < 1e-6,
        "All bits differ: distance = 4"
    );
}

#[test]
fn test_hamming_distance_partial() {
    // Some bits differ
    let a = vec![1.0, 0.0, 1.0, 0.0];
    let b = vec![1.0, 1.0, 0.0, 0.0];
    let distance = DistanceMetric::Hamming.calculate(&a, &b);
    assert!((distance - 2.0).abs() < 1e-6, "2 bits differ: distance = 2");
}

#[test]
fn test_hamming_higher_is_better() {
    // Hamming: lower distance = more similar
    assert!(!DistanceMetric::Hamming.higher_is_better());
}

// =========================================================================
// TDD Tests for Jaccard Similarity (WIS-33)
// =========================================================================

#[test]
fn test_jaccard_similarity_identical() {
    // Identical sets should have similarity 1.0
    let a = vec![1.0, 0.0, 1.0, 1.0];
    let b = vec![1.0, 0.0, 1.0, 1.0];
    let similarity = DistanceMetric::Jaccard.calculate(&a, &b);
    assert!(
        (similarity - 1.0).abs() < 1e-6,
        "Identical sets: similarity = 1.0"
    );
}

#[test]
fn test_jaccard_similarity_disjoint() {
    // Disjoint sets should have similarity 0.0
    let a = vec![1.0, 1.0, 0.0, 0.0];
    let b = vec![0.0, 0.0, 1.0, 1.0];
    let similarity = DistanceMetric::Jaccard.calculate(&a, &b);
    assert!(
        (similarity - 0.0).abs() < 1e-6,
        "Disjoint sets: similarity = 0.0"
    );
}

#[test]
fn test_jaccard_similarity_partial_overlap() {
    // Partial overlap: intersection=2, union=4, similarity=0.5
    let a = vec![1.0, 1.0, 1.0, 0.0];
    let b = vec![1.0, 1.0, 0.0, 1.0];
    let similarity = DistanceMetric::Jaccard.calculate(&a, &b);
    assert!(
        (similarity - 0.5).abs() < 1e-6,
        "Partial overlap: similarity = 0.5"
    );
}

#[test]
fn test_jaccard_similarity_empty_sets() {
    // Both empty sets - defined as 1.0 (identical)
    let a = vec![0.0, 0.0, 0.0, 0.0];
    let b = vec![0.0, 0.0, 0.0, 0.0];
    let similarity = DistanceMetric::Jaccard.calculate(&a, &b);
    assert!(
        (similarity - 1.0).abs() < 1e-6,
        "Empty sets: similarity = 1.0"
    );
}

#[test]
fn test_jaccard_higher_is_better() {
    // Jaccard: higher similarity = more similar
    assert!(DistanceMetric::Jaccard.higher_is_better());
}

// =========================================================================
// TDD Tests for sort_results (QW-1 Refactoring)
// =========================================================================

#[test]
fn test_sort_results_cosine_descending() {
    let mut results = vec![(1, 0.7), (2, 0.9), (3, 0.8)];
    DistanceMetric::Cosine.sort_results(&mut results);
    assert_eq!(results[0].0, 2); // Highest first
    assert_eq!(results[1].0, 3);
    assert_eq!(results[2].0, 1);
}

#[test]
fn test_sort_results_euclidean_ascending() {
    let mut results = vec![(1, 5.0), (2, 2.0), (3, 3.0)];
    DistanceMetric::Euclidean.sort_results(&mut results);
    assert_eq!(results[0].0, 2); // Lowest first
    assert_eq!(results[1].0, 3);
    assert_eq!(results[2].0, 1);
}

#[test]
fn test_sort_results_dot_product_descending() {
    let mut results = vec![(1, 10.0), (2, 30.0), (3, 20.0)];
    DistanceMetric::DotProduct.sort_results(&mut results);
    assert_eq!(results[0].0, 2); // Highest first
}

#[test]
fn test_sort_results_hamming_ascending() {
    let mut results = vec![(1, 4.0), (2, 1.0), (3, 2.0)];
    DistanceMetric::Hamming.sort_results(&mut results);
    assert_eq!(results[0].0, 2); // Lowest first
}

#[test]
fn test_sort_results_jaccard_descending() {
    let mut results = vec![(1, 0.3), (2, 0.9), (3, 0.5)];
    DistanceMetric::Jaccard.sort_results(&mut results);
    assert_eq!(results[0].0, 2); // Highest first
}

#[test]
fn test_sort_results_handles_nan() {
    let mut results = vec![(1, f32::NAN), (2, 0.5), (3, 0.8)];
    // Should not panic with NaN values
    DistanceMetric::Cosine.sort_results(&mut results);
    // NaN ordering is implementation-defined, just verify no panic
}

#[test]
fn test_sort_results_empty() {
    let mut results: Vec<(u64, f32)> = vec![];
    DistanceMetric::Cosine.sort_results(&mut results);
    assert!(results.is_empty());
}
