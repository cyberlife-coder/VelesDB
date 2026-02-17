//! Tests for compound query execution (UNION/INTERSECT/EXCEPT).
//! Plan 08-03: Validates set operations on `SearchResult` vectors.

use super::compound::apply_set_operation;
use crate::point::{Point, SearchResult};
use crate::velesql::SetOperator;

fn make_result(id: u64, score: f32) -> SearchResult {
    SearchResult {
        point: Point {
            id,
            vector: vec![0.1, 0.2],
            payload: Some(serde_json::json!({"id": id})),
        },
        score,
    }
}

#[test]
fn test_union_deduplicates() {
    let left = vec![make_result(1, 0.9), make_result(2, 0.8)];
    let right = vec![make_result(2, 0.7), make_result(3, 0.6)];

    let results = apply_set_operation(left, right, SetOperator::Union);

    assert_eq!(results.len(), 3, "UNION should deduplicate by point.id");
    let ids: Vec<u64> = results.iter().map(|r| r.point.id).collect();
    assert_eq!(ids, vec![1, 2, 3]);

    // First occurrence wins (left's score for id=2)
    let id2 = results.iter().find(|r| r.point.id == 2).unwrap();
    assert!((id2.score - 0.8).abs() < f32::EPSILON);
}

#[test]
fn test_union_merges_different_ids() {
    let left = vec![make_result(1, 0.9)];
    let right = vec![make_result(2, 0.8)];

    let results = apply_set_operation(left, right, SetOperator::Union);

    assert_eq!(results.len(), 2);
    assert_eq!(results[0].point.id, 1);
    assert_eq!(results[1].point.id, 2);
}

#[test]
fn test_union_all_keeps_duplicates() {
    let left = vec![make_result(1, 0.9), make_result(2, 0.8)];
    let right = vec![make_result(2, 0.7), make_result(3, 0.6)];

    let results = apply_set_operation(left, right, SetOperator::UnionAll);

    assert_eq!(results.len(), 4, "UNION ALL should keep all rows");
    let ids: Vec<u64> = results.iter().map(|r| r.point.id).collect();
    assert_eq!(ids, vec![1, 2, 2, 3]);
}

#[test]
fn test_intersect_keeps_common() {
    let left = vec![
        make_result(1, 0.9),
        make_result(2, 0.8),
        make_result(3, 0.7),
    ];
    let right = vec![make_result(2, 0.6), make_result(3, 0.5)];

    let results = apply_set_operation(left, right, SetOperator::Intersect);

    assert_eq!(results.len(), 2, "INTERSECT should keep only common IDs");
    let ids: Vec<u64> = results.iter().map(|r| r.point.id).collect();
    assert_eq!(ids, vec![2, 3]);
}

#[test]
fn test_intersect_disjoint_returns_empty() {
    let left = vec![make_result(1, 0.9), make_result(2, 0.8)];
    let right = vec![make_result(3, 0.7), make_result(4, 0.6)];

    let results = apply_set_operation(left, right, SetOperator::Intersect);

    assert!(
        results.is_empty(),
        "Disjoint sets should produce empty INTERSECT"
    );
}

#[test]
fn test_except_removes_right_from_left() {
    let left = vec![
        make_result(1, 0.9),
        make_result(2, 0.8),
        make_result(3, 0.7),
    ];
    let right = vec![make_result(2, 0.6)];

    let results = apply_set_operation(left, right, SetOperator::Except);

    assert_eq!(results.len(), 2, "EXCEPT should remove right IDs from left");
    let ids: Vec<u64> = results.iter().map(|r| r.point.id).collect();
    assert_eq!(ids, vec![1, 3]);
}

#[test]
fn test_except_with_no_overlap() {
    let left = vec![make_result(1, 0.9), make_result(2, 0.8)];
    let right = vec![make_result(3, 0.7)];

    let results = apply_set_operation(left, right, SetOperator::Except);

    assert_eq!(results.len(), 2, "No overlap means all left rows kept");
}

#[test]
fn test_empty_left_set() {
    let left: Vec<SearchResult> = vec![];
    let right = vec![make_result(1, 0.9)];

    assert!(apply_set_operation(left.clone(), right.clone(), SetOperator::Union).len() == 1);
    assert!(apply_set_operation(left.clone(), right.clone(), SetOperator::Intersect).is_empty());
    assert!(apply_set_operation(left, right, SetOperator::Except).is_empty());
}

#[test]
fn test_empty_right_set() {
    let left = vec![make_result(1, 0.9), make_result(2, 0.8)];
    let right: Vec<SearchResult> = vec![];

    assert_eq!(
        apply_set_operation(left.clone(), right.clone(), SetOperator::Union).len(),
        2
    );
    assert!(apply_set_operation(left.clone(), right.clone(), SetOperator::Intersect).is_empty());
    assert_eq!(
        apply_set_operation(left, right, SetOperator::Except).len(),
        2
    );
}
