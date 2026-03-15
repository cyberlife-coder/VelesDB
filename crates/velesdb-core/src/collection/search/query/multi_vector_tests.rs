//! Tests for multi-vector field retrieval (`get_vector_for_field`).

#![cfg(all(test, feature = "persistence"))]

use crate::collection::types::Collection;
use crate::distance::DistanceMetric;
use crate::point::Point;
use std::path::PathBuf;

/// Helper: create a collection with points that have embedded vectors in payload.
fn setup_multi_vector_collection() -> (tempfile::TempDir, Collection) {
    let dir = tempfile::tempdir().expect("temp dir");
    let col = Collection::create(PathBuf::from(dir.path()), 4, DistanceMetric::Cosine)
        .expect("create collection");

    let points = vec![
        Point {
            id: 1,
            vector: vec![1.0, 0.0, 0.0, 0.0],
            payload: Some(serde_json::json!({
                "title_embedding": [0.5, 0.5, 0.0, 0.0],
                "description": "not a vector"
            })),
            sparse_vectors: None,
        },
        Point {
            id: 2,
            vector: vec![0.0, 1.0, 0.0, 0.0],
            payload: Some(serde_json::json!({
                "title_embedding": [0.0, 0.0, 1.0, 0.0]
            })),
            sparse_vectors: None,
        },
        Point {
            id: 3,
            vector: vec![0.0, 0.0, 1.0, 0.0],
            payload: None, // no payload at all
            sparse_vectors: None,
        },
    ];

    col.upsert(points).expect("upsert");
    (dir, col)
}

// -----------------------------------------------------------------------
// Primary "vector" field
// -----------------------------------------------------------------------

#[test]
fn test_get_vector_for_primary_field() {
    let (_dir, col) = setup_multi_vector_collection();

    let vec = col
        .get_vector_for_field(1, "vector")
        .expect("should succeed");
    assert!(vec.is_some(), "primary vector should exist for point 1");
    assert_eq!(vec.unwrap(), vec![1.0, 0.0, 0.0, 0.0]);
}

// -----------------------------------------------------------------------
// Named vector from payload
// -----------------------------------------------------------------------

#[test]
fn test_get_vector_for_named_payload_field() {
    let (_dir, col) = setup_multi_vector_collection();

    let vec = col
        .get_vector_for_field(1, "title_embedding")
        .expect("should succeed");
    assert!(vec.is_some(), "title_embedding should exist for point 1");
    assert_eq!(vec.unwrap(), vec![0.5, 0.5, 0.0, 0.0]);
}

// -----------------------------------------------------------------------
// Missing field -> Ok(None)
// -----------------------------------------------------------------------

#[test]
fn test_get_vector_for_missing_field_returns_none() {
    let (_dir, col) = setup_multi_vector_collection();

    let vec = col
        .get_vector_for_field(1, "nonexistent_field")
        .expect("should succeed, not error");
    assert!(vec.is_none(), "missing field should return None");
}

// -----------------------------------------------------------------------
// Non-numeric array -> Error
// -----------------------------------------------------------------------

#[test]
fn test_get_vector_for_non_array_field_errors() {
    let (_dir, col) = setup_multi_vector_collection();

    // "description" is a string, not an array.
    let result = col.get_vector_for_field(1, "description");
    assert!(result.is_err(), "non-array field should error");
}

// -----------------------------------------------------------------------
// No payload -> Ok(None)
// -----------------------------------------------------------------------

#[test]
fn test_get_vector_for_field_no_payload_returns_none() {
    let (_dir, col) = setup_multi_vector_collection();

    let vec = col
        .get_vector_for_field(3, "title_embedding")
        .expect("should succeed");
    assert!(vec.is_none(), "point with no payload should return None");
}
