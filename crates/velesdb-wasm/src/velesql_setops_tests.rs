use super::*;

fn row(id: u64) -> QueryResultRow {
    QueryResultRow::build(id, 0.0, None).expect("test: row")
}

#[test]
fn test_union_dedups() {
    let left = vec![row(1), row(2)];
    let right = vec![row(2), row(3)];
    let out = combine(SetOperator::Union, left, right).expect("test: union");
    assert_eq!(out.len(), 3);
}

#[test]
fn test_union_all_keeps_duplicates() {
    let left = vec![row(1), row(2)];
    let right = vec![row(2), row(3)];
    let out = combine(SetOperator::UnionAll, left, right).expect("test: union all");
    assert_eq!(out.len(), 4);
}

#[test]
fn test_intersect_returns_common() {
    let left = vec![row(1), row(2), row(3)];
    let right = vec![row(2), row(3), row(4)];
    let out = combine(SetOperator::Intersect, left, right).expect("test: intersect");
    let ids: Vec<u64> = out.iter().map(QueryResultRow::id).collect();
    assert!(ids.contains(&2));
    assert!(ids.contains(&3));
    assert!(!ids.contains(&1));
}

#[test]
fn test_except_subtracts_right() {
    let left = vec![row(1), row(2), row(3)];
    let right = vec![row(2)];
    let out = combine(SetOperator::Except, left, right).expect("test: except");
    let ids: Vec<u64> = out.iter().map(QueryResultRow::id).collect();
    assert!(ids.contains(&1));
    assert!(ids.contains(&3));
    assert!(!ids.contains(&2));
}

#[test]
fn test_intersect_empty_when_disjoint() {
    let left = vec![row(1)];
    let right = vec![row(2)];
    let out = combine(SetOperator::Intersect, left, right).expect("test: empty intersect");
    assert!(out.is_empty());
}

// --- Finding F11: O(n) dedup preserves first-seen order ---------------

#[test]
fn test_union_preserves_first_seen_order() {
    // First-seen order matters: UNION of [1,2,3] with [2,4] must
    // yield [1,2,3,4] — not [1,4,2,3] (HashMap iteration order).
    let left = vec![row(1), row(2), row(3)];
    let right = vec![row(2), row(4)];
    let out = combine(SetOperator::Union, left, right).expect("test: union order");
    let ids: Vec<u64> = out.iter().map(QueryResultRow::id).collect();
    assert_eq!(ids, vec![1, 2, 3, 4]);
}

#[test]
fn test_intersect_preserves_left_order() {
    let left = vec![row(3), row(1), row(2)];
    let right = vec![row(1), row(2), row(3)];
    let out = combine(SetOperator::Intersect, left, right).expect("test: intersect order");
    let ids: Vec<u64> = out.iter().map(QueryResultRow::id).collect();
    // Order must match the left-hand walk.
    assert_eq!(ids, vec![3, 1, 2]);
}

#[test]
fn test_except_preserves_left_order() {
    let left = vec![row(4), row(2), row(3), row(1)];
    let right = vec![row(2)];
    let out = combine(SetOperator::Except, left, right).expect("test: except order");
    let ids: Vec<u64> = out.iter().map(QueryResultRow::id).collect();
    assert_eq!(ids, vec![4, 3, 1]);
}

#[test]
fn test_dedup_dedups_repeated_rows() {
    let rows = vec![row(1), row(2), row(1), row(3), row(2)];
    let out = combine(
        SetOperator::Union,
        rows,
        // UNION with empty to trigger dedup branch.
        Vec::new(),
    )
    .expect("test: dedup");
    let ids: Vec<u64> = out.iter().map(QueryResultRow::id).collect();
    assert_eq!(ids, vec![1, 2, 3]);
}

#[test]
fn test_setops_handle_large_inputs() {
    // Regression: the previous O(n^2) implementation required ~2s for
    // 2000 rows. The hash-set path handles this in milliseconds. The
    // test here asserts correctness at N=1000; a regression would not
    // fail the assertion but would slow `cargo test` noticeably.
    let left: Vec<QueryResultRow> = (0..1000u64).map(row).collect();
    let right: Vec<QueryResultRow> = (500..1500u64).map(row).collect();
    let out = combine(SetOperator::Union, left, right).expect("test: large-union");
    assert_eq!(out.len(), 1500);
}
