//! Tests for `async_ops` module - Async wrappers for collection operations.

use super::async_ops::*;
use crate::collection::types::Collection;
use crate::point::Point;
use crate::DistanceMetric;
use std::sync::Arc;
use tempfile::TempDir;

fn create_test_collection() -> (TempDir, Arc<Collection>) {
    let dir = TempDir::new().unwrap();
    let collection =
        Collection::create(dir.path().to_path_buf(), 4, DistanceMetric::Cosine).unwrap();
    (dir, Arc::new(collection))
}

#[tokio::test]
async fn test_upsert_bulk_async() {
    let (_dir, collection) = create_test_collection();

    let points: Vec<Point> = (0..100)
        .map(|i| Point::without_payload(i, vec![i as f32; 4]))
        .collect();

    let result = upsert_bulk_async(collection, points).await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), 100);
}

#[tokio::test]
async fn test_upsert_bulk_streaming() {
    let (_dir, collection) = create_test_collection();

    let points: Vec<Point> = (0..500)
        .map(|i| Point::without_payload(i, vec![i as f32; 4]))
        .collect();

    let result = upsert_bulk_streaming(collection, points, 100, None::<fn(usize, usize)>).await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), 500);
}

#[tokio::test]
async fn test_search_async() {
    let (_dir, collection) = create_test_collection();

    let points: Vec<Point> = (0..10)
        .map(|i| Point::without_payload(i, vec![i as f32; 4]))
        .collect();
    collection.upsert(points).unwrap();

    let query = vec![5.0f32; 4];
    let result = search_async(collection, query, 3).await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap().len(), 3);
}

#[tokio::test]
async fn test_flush_async() {
    let (_dir, collection) = create_test_collection();

    let result = flush_async(collection).await;
    assert!(result.is_ok());
}
