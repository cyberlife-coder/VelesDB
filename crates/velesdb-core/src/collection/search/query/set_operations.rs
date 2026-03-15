//! Set operation execution for compound queries (UNION, INTERSECT, EXCEPT).
//!
//! Implements SQL-standard set semantics on `SearchResult` vectors, keyed by
//! point ID. Each operator follows the scoring rules documented below.

use std::collections::HashMap;

use crate::point::SearchResult;
use crate::velesql::SetOperator;

/// Applies a set operator to two result sets.
///
/// Scoring rules per operator:
/// - **Union**: deduplicate by point ID, keep highest score.
/// - **`UnionAll`**: concatenate without deduplication.
/// - **Intersect**: keep only IDs present in both; take the higher score.
/// - **Except**: keep left-side IDs that do not appear in the right side.
///
/// Results are returned sorted by score descending.
pub(crate) fn apply_set_operation(
    left: Vec<SearchResult>,
    right: Vec<SearchResult>,
    operator: SetOperator,
) -> Vec<SearchResult> {
    let mut results = match operator {
        SetOperator::Union => union_dedup(left, right),
        SetOperator::UnionAll => union_all(left, right),
        SetOperator::Intersect => intersect(left, &right),
        SetOperator::Except => except(left, &right),
    };

    // Stable sort by score descending.
    results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    results
}

/// UNION: merge both sides, deduplicate by point ID (keep highest score).
fn union_dedup(left: Vec<SearchResult>, right: Vec<SearchResult>) -> Vec<SearchResult> {
    let mut map: HashMap<u64, SearchResult> = HashMap::with_capacity(left.len() + right.len());

    for result in left {
        map.insert(result.point.id, result);
    }

    for result in right {
        map.entry(result.point.id)
            .and_modify(|existing| {
                if result.score > existing.score {
                    *existing = result.clone();
                }
            })
            .or_insert(result);
    }

    map.into_values().collect()
}

/// UNION ALL: concatenate without deduplication.
fn union_all(mut left: Vec<SearchResult>, right: Vec<SearchResult>) -> Vec<SearchResult> {
    left.extend(right);
    left
}

/// INTERSECT: keep only IDs present in both sides; take the higher score.
fn intersect(left: Vec<SearchResult>, right: &[SearchResult]) -> Vec<SearchResult> {
    let right_map: HashMap<u64, &SearchResult> = right.iter().map(|r| (r.point.id, r)).collect();

    left.into_iter()
        .filter_map(|l| {
            right_map
                .get(&l.point.id)
                .map(|r| if r.score > l.score { (*r).clone() } else { l })
        })
        .collect()
}

/// EXCEPT: keep left-side results whose IDs do not appear in the right side.
fn except(left: Vec<SearchResult>, right: &[SearchResult]) -> Vec<SearchResult> {
    let right_ids: std::collections::HashSet<u64> = right.iter().map(|r| r.point.id).collect();

    left.into_iter()
        .filter(|l| !right_ids.contains(&l.point.id))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::point::{Point, SearchResult};

    fn make_result(id: u64, score: f32) -> SearchResult {
        SearchResult::new(Point::new(id, vec![0.0; 3], None), score)
    }

    #[test]
    fn test_union_dedup_keeps_highest_score() {
        let left = vec![make_result(1, 0.9), make_result(2, 0.5)];
        let right = vec![make_result(2, 0.8), make_result(3, 0.7)];

        let results = apply_set_operation(left, right, SetOperator::Union);

        assert_eq!(results.len(), 3);
        let ids: Vec<u64> = results.iter().map(|r| r.point.id).collect();
        assert!(ids.contains(&1));
        assert!(ids.contains(&2));
        assert!(ids.contains(&3));

        // Point 2 should have score 0.8 (higher of 0.5 and 0.8).
        let point2 = results.iter().find(|r| r.point.id == 2).unwrap();
        assert!((point2.score - 0.8).abs() < f32::EPSILON);
    }

    #[test]
    fn test_union_all_keeps_duplicates() {
        let left = vec![make_result(1, 0.9), make_result(2, 0.5)];
        let right = vec![make_result(2, 0.8), make_result(3, 0.7)];

        let results = apply_set_operation(left, right, SetOperator::UnionAll);

        assert_eq!(results.len(), 4);
    }

    #[test]
    fn test_intersect_keeps_common_ids() {
        let left = vec![make_result(1, 0.9), make_result(2, 0.5)];
        let right = vec![make_result(2, 0.8), make_result(3, 0.7)];

        let results = apply_set_operation(left, right, SetOperator::Intersect);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].point.id, 2);
        // Should keep higher score (0.8).
        assert!((results[0].score - 0.8).abs() < f32::EPSILON);
    }

    #[test]
    fn test_except_removes_right_ids() {
        let left = vec![make_result(1, 0.9), make_result(2, 0.5)];
        let right = vec![make_result(2, 0.8), make_result(3, 0.7)];

        let results = apply_set_operation(left, right, SetOperator::Except);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].point.id, 1);
    }

    #[test]
    fn test_results_sorted_by_score_desc() {
        let left = vec![make_result(1, 0.3), make_result(2, 0.9)];
        let right = vec![make_result(3, 0.6)];

        let results = apply_set_operation(left, right, SetOperator::UnionAll);

        let scores: Vec<f32> = results.iter().map(|r| r.score).collect();
        for window in scores.windows(2) {
            assert!(window[0] >= window[1], "Results not sorted descending");
        }
    }

    #[test]
    fn test_empty_operands() {
        let empty: Vec<SearchResult> = Vec::new();
        let non_empty = vec![make_result(1, 0.5)];

        // UNION with empty left.
        let r = apply_set_operation(Vec::new(), non_empty.clone(), SetOperator::Union);
        assert_eq!(r.len(), 1);

        // INTERSECT with empty left.
        let r = apply_set_operation(Vec::new(), non_empty.clone(), SetOperator::Intersect);
        assert!(r.is_empty());

        // EXCEPT with empty right.
        let r = apply_set_operation(non_empty, empty, SetOperator::Except);
        assert_eq!(r.len(), 1);
    }
}
