//! Tests for `column_store_from_collection` (Plan 08-01 Task 1).

use crate::collection::Collection;
use crate::column_store::from_collection::column_store_from_collection;
use crate::point::Point;
use crate::DistanceMetric;
use serde_json::json;
use tempfile::TempDir;

/// Helper: create a temporary collection with the given dimension.
fn create_temp_collection(dimension: usize) -> (Collection, TempDir) {
    let tmp = TempDir::new().expect("failed to create temp dir");
    let collection =
        Collection::create(tmp.path().to_path_buf(), dimension, DistanceMetric::Cosine)
            .expect("failed to create collection");
    (collection, tmp)
}

/// Helper: create a zero vector of given dimension.
fn zero_vector(dim: usize) -> Vec<f32> {
    vec![0.0; dim]
}

#[test]
fn test_column_store_from_empty_collection() {
    let (collection, _tmp) = create_temp_collection(4);
    let store = column_store_from_collection(&collection, 0).expect("should succeed");
    assert_eq!(store.row_count(), 0);
}

#[test]
fn test_column_store_from_collection_with_payloads() {
    let (collection, _tmp) = create_temp_collection(4);

    // Insert points with varied payload types
    let points = vec![
        Point {
            id: 1,
            vector: zero_vector(4),
            payload: Some(json!({"name": "Alice", "age": 30, "score": 9.5, "active": true})),
        },
        Point {
            id: 2,
            vector: zero_vector(4),
            payload: Some(json!({"name": "Bob", "age": 25, "score": 8.0, "active": false})),
        },
    ];
    collection.upsert(points).expect("upsert failed");

    let store = column_store_from_collection(&collection, 0).expect("should succeed");

    // Should have 2 rows
    assert_eq!(store.row_count(), 2);

    // Should have columns: id, name, age, score, active
    assert!(store.get_column("id").is_some(), "missing 'id' column");
    assert!(store.get_column("name").is_some(), "missing 'name' column");
    assert!(store.get_column("age").is_some(), "missing 'age' column");
    assert!(
        store.get_column("score").is_some(),
        "missing 'score' column"
    );
    assert!(
        store.get_column("active").is_some(),
        "missing 'active' column"
    );
}

#[test]
fn test_column_store_primary_key_is_point_id() {
    let (collection, _tmp) = create_temp_collection(4);

    let points = vec![
        Point {
            id: 42,
            vector: zero_vector(4),
            payload: Some(json!({"category": "tech"})),
        },
        Point {
            id: 99,
            vector: zero_vector(4),
            payload: Some(json!({"category": "science"})),
        },
    ];
    collection.upsert(points).expect("upsert failed");

    let store = column_store_from_collection(&collection, 0).expect("should succeed");

    // PK should be "id"
    assert_eq!(store.primary_key_column(), Some("id"));

    // O(1) lookup by point ID should work
    assert!(store.get_row_idx_by_pk(42).is_some(), "pk 42 not found");
    assert!(store.get_row_idx_by_pk(99).is_some(), "pk 99 not found");
    assert!(
        store.get_row_idx_by_pk(1).is_none(),
        "pk 1 should not exist"
    );

    // Verify value retrieval via PK
    let row_idx = store.get_row_idx_by_pk(42).unwrap();
    let category = store.get_value_as_json("category", row_idx);
    assert_eq!(category, Some(json!("tech")));
}

#[test]
fn test_column_store_max_rows_limit() {
    let (collection, _tmp) = create_temp_collection(4);

    // Insert 10 points
    let points: Vec<Point> = (1..=10)
        .map(|i| Point {
            id: i,
            vector: zero_vector(4),
            payload: Some(json!({"idx": i})),
        })
        .collect();
    collection.upsert(points).expect("upsert failed");

    // Limit to 5 rows
    let store = column_store_from_collection(&collection, 5).expect("should succeed");
    assert!(
        store.row_count() <= 5,
        "should respect max_rows limit, got {}",
        store.row_count()
    );
}

#[test]
fn test_column_store_mixed_types_inferred() {
    let (collection, _tmp) = create_temp_collection(4);

    // Point 1 has all field types
    // Point 2 has a subset (missing "rating")
    let points = vec![
        Point {
            id: 1,
            vector: zero_vector(4),
            payload: Some(json!({"title": "Doc A", "rating": 4.5, "views": 100})),
        },
        Point {
            id: 2,
            vector: zero_vector(4),
            payload: Some(json!({"title": "Doc B", "views": 200})),
        },
    ];
    collection.upsert(points).expect("upsert failed");

    let store = column_store_from_collection(&collection, 0).expect("should succeed");

    assert_eq!(store.row_count(), 2);

    // Point 2 should have null for "rating"
    let row_idx = store.get_row_idx_by_pk(2).unwrap();
    let rating = store.get_value_as_json("rating", row_idx);
    assert_eq!(rating, None, "missing field should be null");

    // Point 2 should have "views" = 200
    let views = store.get_value_as_json("views", row_idx);
    assert_eq!(views, Some(json!(200)));
}

#[test]
fn test_column_store_skips_nested_objects_and_arrays() {
    let (collection, _tmp) = create_temp_collection(4);

    let points = vec![Point {
        id: 1,
        vector: zero_vector(4),
        payload: Some(json!({
            "name": "test",
            "tags": ["a", "b"],
            "meta": {"nested": true}
        })),
    }];
    collection.upsert(points).expect("upsert failed");

    let store = column_store_from_collection(&collection, 0).expect("should succeed");

    // "name" should be present, "tags" and "meta" should NOT be columns
    assert!(store.get_column("name").is_some());
    assert!(store.get_column("tags").is_none(), "arrays not supported");
    assert!(
        store.get_column("meta").is_none(),
        "nested objects not supported"
    );
}

#[test]
fn test_column_store_no_payload_points_skipped() {
    let (collection, _tmp) = create_temp_collection(4);

    let points = vec![
        Point {
            id: 1,
            vector: zero_vector(4),
            payload: None,
        },
        Point {
            id: 2,
            vector: zero_vector(4),
            payload: Some(json!({"name": "Bob"})),
        },
    ];
    collection.upsert(points).expect("upsert failed");

    let store = column_store_from_collection(&collection, 0).expect("should succeed");

    // Point 1 has no payload, but we still create a row with id + null fields
    // Point 2 has payload
    // The ColumnStore should have at least the point with payload
    assert!(store.get_row_idx_by_pk(2).is_some(), "point 2 should exist");
}
