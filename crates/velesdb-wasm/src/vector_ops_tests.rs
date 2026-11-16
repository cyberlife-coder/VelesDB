use super::*;

#[test]
fn test_compute_scores_full() {
    let query = [1.0, 0.0, 0.0, 0.0];
    let ids = vec![1, 2];
    let data = vec![1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0];
    let metric = DistanceMetric::Cosine;

    let scores = compute_scores(&query, &ids, &data, 4, metric);

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

/// A NaN score ranks last, as in core. The previous `partial_cmp` fallback
/// called NaN equal to every score, which is not a total order: the NaN
/// landed wherever the sort happened to leave it.
#[test]
fn test_sort_results_ranks_nan_last_like_core() {
    for higher_is_better in [true, false] {
        for nan in [f32::NAN, -f32::NAN] {
            let mut results = vec![(1, 0.5), (2, nan), (3, 0.9), (4, 0.3)];
            sort_results(&mut results, higher_is_better);
            let order: Vec<u64> = results.iter().map(|r| r.0).collect();
            let expected = if higher_is_better {
                [3, 1, 4, 2]
            } else {
                [4, 1, 3, 2]
            };
            assert_eq!(order, expected, "higher_is_better: {higher_is_better}");
        }
    }
}

#[test]
fn test_filtered_scores_test_a_point_without_payload_like_core() {
    // Core and REST test a payload-less point with a `null` payload, so
    // `is_null` keeps it; the browser used to drop it before the filter ran.
    let filter = velesdb_core::Filter::from_json_value(serde_json::json!(
        {"condition": {"type": "is_null", "field": "tag"}}
    ))
    .expect("test: filter");
    let query = [1.0, 0.0];
    let ids = vec![1, 2];
    let payloads = vec![Some(serde_json::json!({"tag": "x"})), None];
    let data = vec![1.0, 0.0, 0.0, 1.0];

    let kept = compute_filtered_scores(
        &query,
        &ids,
        &payloads,
        &data,
        2,
        DistanceMetric::Cosine,
        |id, payload| filter.matches_point(id, payload),
    );

    let kept_ids: Vec<u64> = kept.iter().map(|(id, _, _)| *id).collect();
    assert_eq!(kept_ids, vec![2]);
}
