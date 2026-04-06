//! Tests for column-store filter operations: eq, gt, lt, range, in,
//! count, bitmap equality/range, and bitmap AND/OR combinators.

#![allow(clippy::cast_possible_wrap)]

use crate::column_store::{ColumnStore, ColumnType, ColumnValue};

/// Helper: creates a column store with `age` (Int), `name` (String), and
/// pushes `count` rows where age=i and name cycles through the given labels.
fn store_with_rows(count: usize, labels: &[&str]) -> ColumnStore {
    let mut store =
        ColumnStore::with_schema(&[("age", ColumnType::Int), ("name", ColumnType::String)]);
    for i in 0..count {
        let label = labels[i % labels.len()];
        let sid = store.string_table_mut().intern(label);
        store.push_row(&[
            ("age", ColumnValue::Int(i as i64)),
            ("name", ColumnValue::String(sid)),
        ]);
    }
    store
}

// ─────────────────────────────────────────────────────────────────────────────
// Equality filters
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn filter_eq_int_finds_matching_rows() {
    let store = store_with_rows(10, &["a"]);
    let result = store.filter_eq_int("age", 5);
    assert_eq!(result, vec![5]);
}

#[test]
fn filter_eq_int_nonexistent_column_returns_empty() {
    let store = store_with_rows(5, &["a"]);
    assert!(store.filter_eq_int("missing", 0).is_empty());
}

#[test]
fn filter_eq_string_finds_matching_rows() {
    let store = store_with_rows(6, &["x", "y"]);
    // Rows 0,2,4 have "x"; rows 1,3,5 have "y"
    let result = store.filter_eq_string("name", "y");
    assert_eq!(result, vec![1, 3, 5]);
}

#[test]
fn filter_eq_string_unknown_value_returns_empty() {
    let store = store_with_rows(4, &["a"]);
    assert!(store.filter_eq_string("name", "zzz").is_empty());
}

// ─────────────────────────────────────────────────────────────────────────────
// Range filters: gt, lt, range
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn filter_gt_int_exclusive() {
    let store = store_with_rows(10, &["a"]);
    let result = store.filter_gt_int("age", 7);
    assert_eq!(result, vec![8, 9]);
}

#[test]
fn filter_lt_int_exclusive() {
    let store = store_with_rows(10, &["a"]);
    let result = store.filter_lt_int("age", 3);
    assert_eq!(result, vec![0, 1, 2]);
}

#[test]
fn filter_range_int_exclusive_bounds() {
    let store = store_with_rows(10, &["a"]);
    // low=2, high=6 => matches 3,4,5
    let result = store.filter_range_int("age", 2, 6);
    assert_eq!(result, vec![3, 4, 5]);
}

// ─────────────────────────────────────────────────────────────────────────────
// IN filter
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn filter_in_string_matches_subset() {
    let store = store_with_rows(9, &["a", "b", "c"]);
    let result = store.filter_in_string("name", &["a", "c"]);
    // a=0,3,6  c=2,5,8
    assert_eq!(result, vec![0, 2, 3, 5, 6, 8]);
}

#[test]
fn filter_in_string_no_matches() {
    let store = store_with_rows(4, &["a"]);
    assert!(store.filter_in_string("name", &["z"]).is_empty());
}

// ─────────────────────────────────────────────────────────────────────────────
// Deleted rows are excluded
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn filter_excludes_deleted_rows() {
    let mut store = store_with_rows(5, &["a"]);
    // Delete row 2
    store.deleted_rows.insert(2);
    if let Ok(idx) = u32::try_from(2_usize) {
        store.deletion_bitmap.insert(idx);
    }
    let result = store.filter_eq_int("age", 2);
    assert!(result.is_empty(), "deleted row must be excluded");
}

// ─────────────────────────────────────────────────────────────────────────────
// Count operations
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn count_eq_int_matches() {
    let store = store_with_rows(10, &["a"]);
    assert_eq!(store.count_eq_int("age", 3), 1);
    assert_eq!(store.count_eq_int("age", 999), 0);
}

#[test]
fn count_eq_string_matches() {
    let store = store_with_rows(6, &["x", "y"]);
    assert_eq!(store.count_eq_string("name", "x"), 3);
}

// ─────────────────────────────────────────────────────────────────────────────
// Bitmap filters and combinators
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn bitmap_eq_int_matches_same_as_vec() {
    let store = store_with_rows(10, &["a"]);
    let bitmap = store.filter_eq_int_bitmap("age", 5);
    assert!(bitmap.contains(5));
    assert_eq!(bitmap.len(), 1);
}

#[test]
fn bitmap_range_int() {
    let store = store_with_rows(10, &["a"]);
    let bitmap = store.filter_range_int_bitmap("age", 2, 6);
    assert_eq!(bitmap.len(), 3); // 3,4,5
}

#[test]
fn bitmap_and_combinator() {
    let store = store_with_rows(10, &["a"]);
    let a = store.filter_range_int_bitmap("age", 0, 8); // 1..7
    let b = store.filter_range_int_bitmap("age", 4, 10); // 5..9
    let combined = ColumnStore::bitmap_and(&a, &b);
    // Intersection: 5,6,7
    assert_eq!(combined.len(), 3);
    assert!(combined.contains(5));
    assert!(combined.contains(6));
    assert!(combined.contains(7));
}

#[test]
fn bitmap_or_combinator() {
    let store = store_with_rows(10, &["a"]);
    let a = store.filter_eq_int_bitmap("age", 1);
    let b = store.filter_eq_int_bitmap("age", 9);
    let combined = ColumnStore::bitmap_or(&a, &b);
    assert_eq!(combined.len(), 2);
    assert!(combined.contains(1));
    assert!(combined.contains(9));
}

// ─────────────────────────────────────────────────────────────────────────────
// GeoPoint filter tests
// ─────────────────────────────────────────────────────────────────────────────

use crate::column_store::filter::{CompareOp, GeoBboxParams, GeoDistanceParams};

/// Helper: creates a column store with a GeoPoint column and pushes test rows.
fn geo_store() -> ColumnStore {
    let mut store = ColumnStore::with_schema(&[("location", ColumnType::GeoPoint)]);
    // Paris
    store.push_row(&[("location", ColumnValue::GeoPoint(48.8566, 2.3522))]);
    // London
    store.push_row(&[("location", ColumnValue::GeoPoint(51.5074, -0.1278))]);
    // NYC
    store.push_row(&[("location", ColumnValue::GeoPoint(40.7128, -74.0060))]);
    // Null
    store.push_row(&[("location", ColumnValue::Null)]);
    store
}

#[test]
fn filter_geo_distance_finds_nearby_points() {
    let store = geo_store();
    let params = GeoDistanceParams {
        column: "location",
        lat: 48.8566,
        lng: 2.3522,
        operator: CompareOp::Lt,
        threshold: 500_000.0, // 500 km
    };
    let result = store.filter_geo_distance(&params);
    // Paris (0) is 0m away, London (1) is ~343km away — both within 500km
    assert!(result.contains(&0));
    assert!(result.contains(&1));
    // NYC (2) is too far, null (3) excluded
    assert!(!result.contains(&2));
    assert!(!result.contains(&3));
}

#[test]
fn filter_geo_distance_nonexistent_column_returns_empty() {
    let store = geo_store();
    let params = GeoDistanceParams {
        column: "nonexistent",
        lat: 0.0,
        lng: 0.0,
        operator: CompareOp::Lt,
        threshold: 1_000_000.0,
    };
    assert!(store.filter_geo_distance(&params).is_empty());
}

#[test]
fn filter_geo_distance_non_geopoint_column_returns_empty() {
    let store = store_with_rows(5, &["a"]);
    let params = GeoDistanceParams {
        column: "age",
        lat: 0.0,
        lng: 0.0,
        operator: CompareOp::Lt,
        threshold: 1_000_000.0,
    };
    assert!(store.filter_geo_distance(&params).is_empty());
}

#[test]
fn filter_geo_bbox_finds_points_in_box() {
    let store = geo_store();
    let params = GeoBboxParams {
        column: "location",
        lat_min: 48.0,
        lng_min: 2.0,
        lat_max: 49.0,
        lng_max: 3.0,
    };
    let result = store.filter_geo_bbox(&params);
    assert!(result.contains(&0)); // Paris
    assert!(!result.contains(&1)); // London outside
    assert!(!result.contains(&3)); // Null excluded
}

#[test]
fn filter_geo_bbox_inverted_returns_empty() {
    let store = geo_store();
    let params = GeoBboxParams {
        column: "location",
        lat_min: 49.0,
        lng_min: 3.0,
        lat_max: 48.0,
        lng_max: 2.0,
    };
    assert!(store.filter_geo_bbox(&params).is_empty());
}

#[test]
fn filter_geo_distance_bitmap_matches_vec() {
    let store = geo_store();
    let params = GeoDistanceParams {
        column: "location",
        lat: 48.8566,
        lng: 2.3522,
        operator: CompareOp::Lt,
        threshold: 500_000.0,
    };
    let vec_result = store.filter_geo_distance(&params);
    let bitmap_result = store.filter_geo_distance_bitmap(&params);
    let bitmap_vec: Vec<usize> = bitmap_result.iter().map(|i| i as usize).collect();
    assert_eq!(vec_result, bitmap_vec);
}

#[test]
fn filter_geo_bbox_bitmap_matches_vec() {
    let store = geo_store();
    let params = GeoBboxParams {
        column: "location",
        lat_min: 40.0,
        lng_min: -75.0,
        lat_max: 52.0,
        lng_max: 3.0,
    };
    let vec_result = store.filter_geo_bbox(&params);
    let bitmap_result = store.filter_geo_bbox_bitmap(&params);
    let bitmap_vec: Vec<usize> = bitmap_result.iter().map(|i| i as usize).collect();
    assert_eq!(vec_result, bitmap_vec);
}
