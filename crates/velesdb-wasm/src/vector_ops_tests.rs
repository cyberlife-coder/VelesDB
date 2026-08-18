use super::*;

#[test]
fn test_compute_scores_full() {
    let query = [1.0, 0.0, 0.0, 0.0];
    let ids = vec![1, 2];
    let data = vec![1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0];
    let metric = DistanceMetric::Cosine;

    let scores = compute_scores(
        &query,
        &ids,
        &data,
        &[],
        &[],
        &[],
        &[],
        4,
        metric,
        StorageMode::Full,
    );

    assert_eq!(scores.len(), 2);
    assert_eq!(scores[0].0, 1);
    assert!((scores[0].1 - 1.0).abs() < 0.01);
}

#[test]
fn test_sort_results_higher() {
    let mut results = vec![(1, 0.5), (2, 0.9), (3, 0.3)];
    sort_results(&mut results, true);
    assert_eq!(results[0].0, 2);
}

#[test]
fn test_sort_results_lower() {
    let mut results = vec![(1, 0.5), (2, 0.9), (3, 0.3)];
    sort_results(&mut results, false);
    assert_eq!(results[0].0, 3);
}
