//! End-to-end execution tests for UNION/INTERSECT/EXCEPT (EPIC-040 US-006).
//!
//! These tests create collections, insert data, run compound VelesQL queries
//! via `execute_query_str`, and verify correct result sets.

use crate::collection::types::Collection;
use crate::distance::DistanceMetric;
use crate::point::Point;
use std::collections::HashMap;
use tempfile::TempDir;

/// Helper: create a collection and insert points with the given IDs and categories.
fn make_collection_with_data(
    dir: &std::path::Path,
    name: &str,
    ids_and_categories: &[(u64, &str)],
) -> Collection {
    let col = Collection::create(dir.join(name), 3, DistanceMetric::Cosine).expect("create failed");
    let points: Vec<Point> = ids_and_categories
        .iter()
        .map(|&(id, cat)| {
            #[allow(clippy::cast_precision_loss)]
            Point::new(
                id,
                vec![id as f32 / 100.0, 0.1, 0.1],
                Some(serde_json::json!({ "category": cat, "idx": id })),
            )
        })
        .collect();
    col.upsert(points).expect("upsert failed");
    col
}

// ─── UNION ────────────────────────────────────────────────────────────────────

#[test]
fn test_union_execution_deduplicates() {
    let dir = TempDir::new().unwrap();
    // Points: IDs 1,2,3 with category='a', and IDs 2,3,4 with mixed categories.
    // IDs 2 and 3 overlap between the two WHERE conditions.
    let col = make_collection_with_data(
        dir.path(),
        "union_col",
        &[(1, "a"), (2, "a"), (3, "a"), (4, "b"), (5, "b")],
    );
    let params = HashMap::new();

    let sql = "SELECT * FROM union_col WHERE category = 'a' \
               UNION \
               SELECT * FROM union_col WHERE idx > 2 LIMIT 100";
    let results = col
        .execute_query_str(sql, &params)
        .expect("UNION execution failed");

    // category='a' -> {1, 2, 3}; idx > 2 -> {3, 4, 5}; UNION -> {1, 2, 3, 4, 5}
    let ids: std::collections::HashSet<u64> = results.iter().map(|r| r.point.id).collect();
    assert_eq!(ids.len(), 5, "UNION should produce 5 unique points");
    assert!(ids.contains(&1));
    assert!(ids.contains(&4));
    assert!(ids.contains(&5));
}

#[test]
fn test_union_all_execution_keeps_duplicates() {
    let dir = TempDir::new().unwrap();
    let col =
        make_collection_with_data(dir.path(), "union_all_col", &[(1, "a"), (2, "a"), (3, "b")]);
    let params = HashMap::new();

    let sql = "SELECT * FROM union_all_col WHERE category = 'a' \
               UNION ALL \
               SELECT * FROM union_all_col WHERE idx >= 2 LIMIT 100";
    let results = col
        .execute_query_str(sql, &params)
        .expect("UNION ALL execution failed");

    // category='a' -> {1, 2}; idx >= 2 -> {2, 3}; UNION ALL -> [1, 2, 2, 3] = 4 rows
    assert_eq!(
        results.len(),
        4,
        "UNION ALL should keep duplicates (4 rows total)"
    );
}

// ─── INTERSECT ────────────────────────────────────────────────────────────────

#[test]
fn test_intersect_execution() {
    let dir = TempDir::new().unwrap();
    let col = make_collection_with_data(
        dir.path(),
        "intersect_col",
        &[(1, "a"), (2, "a"), (3, "b"), (4, "b")],
    );
    let params = HashMap::new();

    let sql = "SELECT * FROM intersect_col WHERE category = 'a' \
               INTERSECT \
               SELECT * FROM intersect_col WHERE idx >= 2 LIMIT 100";
    let results = col
        .execute_query_str(sql, &params)
        .expect("INTERSECT execution failed");

    // category='a' -> {1, 2}; idx >= 2 -> {2, 3, 4}; INTERSECT -> {2}
    assert_eq!(results.len(), 1, "INTERSECT should produce only common IDs");
    assert_eq!(results[0].point.id, 2);
}

#[test]
fn test_intersect_empty_result() {
    let dir = TempDir::new().unwrap();
    let col = make_collection_with_data(dir.path(), "intersect_empty", &[(1, "a"), (2, "b")]);
    let params = HashMap::new();

    let sql = "SELECT * FROM intersect_empty WHERE category = 'a' \
               INTERSECT \
               SELECT * FROM intersect_empty WHERE category = 'b' LIMIT 100";
    let results = col
        .execute_query_str(sql, &params)
        .expect("INTERSECT execution failed");

    assert!(
        results.is_empty(),
        "INTERSECT of disjoint sets should be empty"
    );
}

// ─── EXCEPT ──────────────────────────────────────────────────────────────────

#[test]
fn test_except_execution() {
    let dir = TempDir::new().unwrap();
    let col = make_collection_with_data(
        dir.path(),
        "except_col",
        &[(1, "a"), (2, "a"), (3, "b"), (4, "b")],
    );
    let params = HashMap::new();

    let sql = "SELECT * FROM except_col WHERE category = 'a' \
               EXCEPT \
               SELECT * FROM except_col WHERE idx = 2 LIMIT 100";
    let results = col
        .execute_query_str(sql, &params)
        .expect("EXCEPT execution failed");

    // category='a' -> {1, 2}; idx = 2 -> {2}; EXCEPT -> {1}
    assert_eq!(
        results.len(),
        1,
        "EXCEPT should remove matching right-side IDs"
    );
    assert_eq!(results[0].point.id, 1);
}

#[test]
fn test_except_nothing_removed() {
    let dir = TempDir::new().unwrap();
    let col = make_collection_with_data(dir.path(), "except_noop", &[(1, "a"), (2, "a"), (3, "b")]);
    let params = HashMap::new();

    let sql = "SELECT * FROM except_noop WHERE category = 'a' \
               EXCEPT \
               SELECT * FROM except_noop WHERE category = 'b' LIMIT 100";
    let results = col
        .execute_query_str(sql, &params)
        .expect("EXCEPT execution failed");

    // category='a' -> {1, 2}; category='b' -> {3}; EXCEPT -> {1, 2}
    let ids: std::collections::HashSet<u64> = results.iter().map(|r| r.point.id).collect();
    assert_eq!(
        ids.len(),
        2,
        "EXCEPT with no overlap should keep all left rows"
    );
    assert!(ids.contains(&1));
    assert!(ids.contains(&2));
}

// ─── Large dataset (>10 per operand) — regression for default LIMIT bug ──────

#[test]
fn test_union_large_dataset_not_truncated() {
    let dir = TempDir::new().unwrap();
    // 20 points with category='a' (ids 1..=20) and 20 with category='b' (ids 21..=40)
    let mut data: Vec<(u64, &str)> = (1..=20).map(|id| (id, "a")).collect();
    data.extend((21..=40).map(|id| (id, "b")));
    let col = make_collection_with_data(dir.path(), "union_large", &data);
    let params = HashMap::new();

    let sql = "SELECT * FROM union_large WHERE category = 'a' \
               UNION \
               SELECT * FROM union_large WHERE category = 'b' LIMIT 100";
    let results = col
        .execute_query_str(sql, &params)
        .expect("UNION large execution failed");

    // Both operands have 20 rows each, no overlap → UNION = 40 rows.
    assert_eq!(
        results.len(),
        40,
        "UNION must return all 40 points, not be truncated by default LIMIT"
    );
}

// ─── Simple SELECT still works ───────────────────────────────────────────────

#[test]
fn test_non_compound_query_unaffected() {
    let dir = TempDir::new().unwrap();
    let col = make_collection_with_data(dir.path(), "simple_col", &[(1, "a"), (2, "b")]);
    let params = HashMap::new();

    let sql = "SELECT * FROM simple_col LIMIT 10";
    let results = col
        .execute_query_str(sql, &params)
        .expect("Simple SELECT should still work");
    assert_eq!(results.len(), 2);
}
