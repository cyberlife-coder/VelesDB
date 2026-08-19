use super::*;
use crate::point::Point;

fn result(id: u64, score: f32) -> SearchResult {
    SearchResult::new(
        Point {
            id,
            vector: vec![],
            payload: None,
            sparse_vectors: None,
        },
        score,
    )
}

/// Bounded top-k returns the same ids/order as full sort+truncate
/// (higher-is-better direction).
#[test]
fn test_bounded_top_k_matches_full_sort_higher_better() {
    let scores = [0.1f32, 0.9, 0.5, 0.95, 0.3, 0.7];
    let mut topk = BoundedTopK::new(3, true);
    for (i, s) in scores.iter().enumerate() {
        topk.offer(result(i as u64, *s));
    }
    let got: Vec<(u64, f32)> = topk
        .into_sorted_vec()
        .iter()
        .map(|r| (r.point.id, r.score))
        .collect();

    // Reference: full sort desc + truncate(3).
    let mut reference: Vec<(u64, f32)> = scores
        .iter()
        .enumerate()
        .map(|(i, s)| (i as u64, *s))
        .collect();
    reference.sort_by(|a, b| b.1.total_cmp(&a.1));
    reference.truncate(3);

    assert_eq!(got, reference);
}

/// Lower-is-better (distance) direction keeps the smallest scores.
#[test]
fn test_bounded_top_k_matches_full_sort_lower_better() {
    let scores = [5.0f32, 1.0, 3.0, 0.5, 9.0, 2.0];
    let mut topk = BoundedTopK::new(2, false);
    for (i, s) in scores.iter().enumerate() {
        topk.offer(result(i as u64, *s));
    }
    let got: Vec<u64> = topk.into_sorted_vec().iter().map(|r| r.point.id).collect();
    assert_eq!(got, vec![3, 1]); // 0.5 then 1.0
}

/// Equal scores keep first-seen (insertion) order.
#[test]
fn test_bounded_top_k_ties_keep_insertion_order() {
    let mut topk = BoundedTopK::new(2, true);
    topk.offer(result(10, 0.5));
    topk.offer(result(20, 0.5));
    topk.offer(result(30, 0.5));
    let got: Vec<u64> = topk.into_sorted_vec().iter().map(|r| r.point.id).collect();
    assert_eq!(got, vec![10, 20]);
}
