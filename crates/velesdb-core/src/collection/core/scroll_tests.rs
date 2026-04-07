#![cfg(all(test, feature = "persistence"))]

use crate::collection::types::Collection;
use crate::distance::DistanceMetric;
use crate::point::Point;
use serde_json::json;
use std::path::PathBuf;

/// Helper: creates a temporary collection with the given dimension.
fn temp_collection(dim: usize) -> (tempfile::TempDir, Collection) {
    let dir = tempfile::tempdir().expect("temp dir");
    let col = Collection::create(PathBuf::from(dir.path()), dim, DistanceMetric::Cosine)
        .expect("collection created");
    (dir, col)
}

#[test]
fn test_scroll_batch_zero_size_returns_error() {
    let (_dir, col) = temp_collection(4);
    let result = col.scroll_batch(None, 0, None);
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("batch_size must be greater than 0"),
        "unexpected error: {err_msg}"
    );
}

#[test]
fn test_scroll_batch_cursor_none_starts_beginning() {
    let (_dir, col) = temp_collection(2);
    col.upsert(vec![
        Point::new(10, vec![1.0, 0.0], Some(json!({"k": "a"}))),
        Point::new(20, vec![0.0, 1.0], Some(json!({"k": "b"}))),
        Point::new(30, vec![1.0, 1.0], Some(json!({"k": "c"}))),
    ])
    .expect("upsert");

    let batch = col.scroll_batch(None, 10, None).expect("scroll");
    let ids: Vec<u64> = batch.points.iter().map(|p| p.id).collect();
    assert_eq!(ids, vec![10, 20, 30]);
}

#[test]
fn test_scroll_batch_exhausted_returns_none_cursor() {
    let (_dir, col) = temp_collection(2);
    col.upsert(vec![
        Point::new(1, vec![1.0, 0.0], None),
        Point::new(2, vec![0.0, 1.0], None),
    ])
    .expect("upsert");

    // Fetch all in one batch
    let batch = col.scroll_batch(None, 10, None).expect("scroll");
    assert_eq!(batch.points.len(), 2);
    assert!(batch.next_cursor.is_some());

    // Resume from last cursor — should be empty
    let batch2 = col
        .scroll_batch(batch.next_cursor, 10, None)
        .expect("scroll");
    assert!(batch2.points.is_empty());
    assert!(batch2.next_cursor.is_none());
}

#[test]
fn test_scroll_batch_empty_collection() {
    let (_dir, col) = temp_collection(4);
    let batch = col.scroll_batch(None, 10, None).expect("scroll");
    assert!(batch.points.is_empty());
    assert!(batch.next_cursor.is_none());
}

#[test]
fn test_scroll_batch_partial_last_batch() {
    let (_dir, col) = temp_collection(2);
    col.upsert(vec![
        Point::new(1, vec![1.0, 0.0], None),
        Point::new(2, vec![0.0, 1.0], None),
        Point::new(3, vec![1.0, 1.0], None),
    ])
    .expect("upsert");

    // batch_size=2: first batch gets 2, second gets 1
    let b1 = col.scroll_batch(None, 2, None).expect("scroll");
    assert_eq!(b1.points.len(), 2);
    let ids1: Vec<u64> = b1.points.iter().map(|p| p.id).collect();
    assert_eq!(ids1, vec![1, 2]);

    let b2 = col.scroll_batch(b1.next_cursor, 2, None).expect("scroll");
    assert_eq!(b2.points.len(), 1);
    assert_eq!(b2.points[0].id, 3);

    // Third call: exhausted
    let b3 = col.scroll_batch(b2.next_cursor, 2, None).expect("scroll");
    assert!(b3.points.is_empty());
    assert!(b3.next_cursor.is_none());
}
