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

use crate::column_store::filter_geo::{CompareOp, GeoBboxParams, GeoDistanceParams};

/// Helper: creates a column store with a GeoPoint column and pushes test rows.
fn geo_store() -> ColumnStore {
    let mut store = ColumnStore::with_schema(&[("location", ColumnType::GeoPoint)]);
    // Paris
    store.push_row(&[("location", ColumnValue::GeoPoint(PARIS.0, PARIS.1))]);
    // London
    store.push_row(&[("location", ColumnValue::GeoPoint(LONDON.0, LONDON.1))]);
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

// Paris is row 0 and London row 1 of `geo_store`; the reference point is
// Paris, so London is the row these tests watch.
const PARIS: (f64, f64) = (48.8566, 2.3522);
const LONDON: (f64, f64) = (51.5074, -0.1278);
const LONDON_ROW: usize = 1;

fn london_distance_from_paris() -> f64 {
    crate::geo_distance::great_circle_distance_m(PARIS.0, PARIS.1, LONDON.0, LONDON.1)
        .expect("test: valid coordinates")
}

fn from_paris(operator: CompareOp, threshold: f64) -> GeoDistanceParams<'static> {
    GeoDistanceParams {
        column: "location",
        lat: PARIS.0,
        lng: PARIS.1,
        operator,
        threshold,
    }
}

#[test]
fn filter_geo_distance_eq_is_to_the_millimetre() {
    let store = geo_store();
    let dist = london_distance_from_paris();
    let within = dist + crate::geo_distance::DISTANCE_EQUALITY_RESOLUTION_M / 2.0;
    let beyond = dist + crate::geo_distance::DISTANCE_EQUALITY_RESOLUTION_M * 2.0;

    assert!(store
        .filter_geo_distance(&from_paris(CompareOp::Eq, within))
        .contains(&LONDON_ROW));
    assert!(!store
        .filter_geo_distance(&from_paris(CompareOp::NotEq, within))
        .contains(&LONDON_ROW));
    assert!(!store
        .filter_geo_distance(&from_paris(CompareOp::Eq, beyond))
        .contains(&LONDON_ROW));
    assert!(store
        .filter_geo_distance(&from_paris(CompareOp::NotEq, beyond))
        .contains(&LONDON_ROW));
}

// #2310: Haversine gave NaN for this exactly antipodal pair, so the row fell
// out of every comparison.
#[test]
fn filter_geo_distance_near_the_antipode_is_a_real_distance() {
    let mut store = ColumnStore::with_schema(&[("location", ColumnType::GeoPoint)]);
    store.push_row(&[("location", ColumnValue::GeoPoint(2.5, -120.0))]);
    let params = GeoDistanceParams {
        column: "location",
        lat: -2.5,
        lng: 60.0,
        operator: CompareOp::Gt,
        threshold: 20_000_000.0,
    };
    assert_eq!(store.filter_geo_distance(&params), vec![0]);
    assert_eq!(store.filter_geo_distance_bitmap(&params).len(), 1);
}

#[test]
fn filter_geo_distance_off_globe_reference_or_nan_threshold_matches_no_row() {
    let store = geo_store();
    let off_globe = [(f64::NAN, PARIS.1), (PARIS.0, f64::NAN), (90.5, PARIS.1)];
    for (lat, lng) in off_globe {
        let params = GeoDistanceParams {
            lat,
            lng,
            ..from_paris(CompareOp::NotEq, 0.0)
        };
        assert!(
            store.filter_geo_distance(&params).is_empty(),
            "({lat}, {lng})"
        );
        assert!(
            store.filter_geo_distance_bitmap(&params).is_empty(),
            "({lat}, {lng})"
        );
    }
    let nan_threshold = from_paris(CompareOp::NotEq, f64::NAN);
    assert!(store.filter_geo_distance(&nan_threshold).is_empty());
    assert!(store.filter_geo_distance_bitmap(&nan_threshold).is_empty());
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

// ─────────────────────────────────────────────────────────────────────────────
// IN bitmap filters (Issue #512 — Task 4)
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_filter_in_string_bitmap_nominal() {
    let store = store_with_rows(9, &["a", "b", "c"]);
    let bitmap = store.filter_in_string_bitmap("name", &["a", "c"]);
    // a=0,3,6  c=2,5,8
    let ids: Vec<u32> = bitmap.iter().collect();
    assert_eq!(ids, vec![0, 2, 3, 5, 6, 8]);
}

#[test]
fn test_filter_in_string_bitmap_missing_column() {
    let store = store_with_rows(5, &["a"]);
    let bitmap = store.filter_in_string_bitmap("nonexistent", &["a"]);
    assert!(bitmap.is_empty());
}

#[test]
fn test_filter_in_string_bitmap_excludes_deleted() {
    let mut store = store_with_rows(6, &["a", "b"]);
    // Rows: 0=a, 1=b, 2=a, 3=b, 4=a, 5=b
    // Delete rows 0 and 4 (both "a")
    store.deletion_bitmap.insert(0);
    store.deletion_bitmap.insert(4);

    let bitmap = store.filter_in_string_bitmap("name", &["a"]);
    // Only row 2 should remain (row 0 and 4 deleted)
    let ids: Vec<u32> = bitmap.iter().collect();
    assert_eq!(ids, vec![2]);
}

#[test]
fn test_filter_in_string_bitmap_no_matches() {
    let store = store_with_rows(4, &["a"]);
    let bitmap = store.filter_in_string_bitmap("name", &["z", "q"]);
    assert!(bitmap.is_empty());
}

#[test]
fn test_filter_in_int_bitmap_nominal() {
    let store = store_with_rows(10, &["a"]);
    // age column: 0,1,2,...,9
    let bitmap = store.filter_in_int_bitmap("age", &[2, 5, 7]);
    let ids: Vec<u32> = bitmap.iter().collect();
    assert_eq!(ids, vec![2, 5, 7]);
}

#[test]
fn test_filter_in_int_bitmap_type_mismatch() {
    let store = store_with_rows(5, &["a"]);
    // "name" is a String column, not Int
    let bitmap = store.filter_in_int_bitmap("name", &[1, 2]);
    assert!(bitmap.is_empty());
}

#[test]
fn test_filter_in_int_bitmap_excludes_deleted() {
    let mut store = store_with_rows(6, &["a"]);
    // age column: 0,1,2,3,4,5
    // Delete row 2
    store.deletion_bitmap.insert(2);

    let bitmap = store.filter_in_int_bitmap("age", &[1, 2, 3]);
    // Row 2 deleted, so only rows 1 and 3
    let ids: Vec<u32> = bitmap.iter().collect();
    assert_eq!(ids, vec![1, 3]);
}
