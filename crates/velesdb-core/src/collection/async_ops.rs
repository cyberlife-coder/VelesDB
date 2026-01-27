//! Async wrappers for blocking Collection operations.
//!
//! EPIC-034/US-005: Provides async bulk insert API using `spawn_blocking`
//! to avoid blocking the async executor during I/O-intensive operations.
//!
//! # Why spawn_blocking?
//!
//! Collection operations like bulk insert involve:
//! - Memory-mapped file writes (blocking syscalls)
//! - HNSW index updates (CPU-intensive)
//! - Payload storage writes (blocking I/O)
//!
//! These operations can stall the async runtime. This module wraps them
//! to run on Tokio's blocking thread pool.
//!
//! # Usage
//!
//! ```rust,ignore
//! use velesdb_core::{Collection, Point};
//! use velesdb_core::collection::async_ops;
//! use std::sync::Arc;
//!
//! async fn bulk_import(collection: Arc<Collection>) -> Result<usize, Error> {
//!     let points: Vec<Point> = generate_points();
//!     async_ops::upsert_bulk_async(collection, points).await
//! }
//! ```

use std::sync::Arc;

use crate::error::{Error, Result};
use crate::point::Point;

use super::types::Collection;

/// Asynchronously inserts or updates multiple points in bulk.
///
/// Wraps `Collection::upsert_bulk()` in `spawn_blocking` to avoid
/// blocking the async executor during I/O and HNSW operations.
///
/// # Performance
///
/// - Uses parallel HNSW insertion internally
/// - Single batch WAL write for efficiency
/// - Benchmark: 25-30 Kvec/s on 768D vectors
///
/// # Arguments
///
/// * `collection` - Arc-wrapped collection instance
/// * `points` - Vector of points to insert/update
///
/// # Errors
///
/// Returns an error if any point has a mismatched dimension or if
/// the blocking task panics.
pub async fn upsert_bulk_async(collection: Arc<Collection>, points: Vec<Point>) -> Result<usize> {
    tokio::task::spawn_blocking(move || collection.upsert_bulk(&points))
        .await
        .map_err(|e| Error::Internal(format!("Task join error: {e}")))?
}

/// Asynchronously inserts or updates points with streaming support.
///
/// Processes points in chunks to provide progress updates and
/// avoid memory pressure for very large imports.
///
/// # Arguments
///
/// * `collection` - Arc-wrapped collection instance
/// * `points` - Vector of points to insert/update
/// * `chunk_size` - Number of points per batch (default: 10000)
/// * `on_progress` - Optional callback for progress updates
///
/// # Returns
///
/// Total number of points successfully inserted.
///
/// # Errors
///
/// Returns an error if any insert operation fails.
pub async fn upsert_bulk_streaming<F>(
    collection: Arc<Collection>,
    points: Vec<Point>,
    chunk_size: usize,
    mut on_progress: Option<F>,
) -> Result<usize>
where
    F: FnMut(usize, usize) + Send + 'static,
{
    let total = points.len();
    let chunk_size = chunk_size.max(100); // Minimum 100 per chunk
    let mut inserted = 0;

    for (chunk_idx, chunk) in points.chunks(chunk_size).enumerate() {
        let chunk_vec: Vec<Point> = chunk.to_vec();
        let coll = Arc::clone(&collection);

        let count = tokio::task::spawn_blocking(move || coll.upsert_bulk(&chunk_vec))
            .await
            .map_err(|e| Error::Internal(format!("Task join error: {e}")))?;

        inserted += count?;

        // Report progress
        if let Some(ref mut callback) = on_progress {
            callback(inserted, total);
        }

        // Yield to allow other async tasks to run
        if chunk_idx % 10 == 0 {
            tokio::task::yield_now().await;
        }
    }

    Ok(inserted)
}

/// Asynchronously flushes the collection to disk.
///
/// Wraps `Collection::flush()` in `spawn_blocking` to avoid blocking
/// the async executor during disk sync operations.
///
/// # Errors
///
/// Returns an error if file operations fail or if the blocking task panics.
pub async fn flush_async(collection: Arc<Collection>) -> Result<()> {
    tokio::task::spawn_blocking(move || collection.flush())
        .await
        .map_err(|e| Error::Internal(format!("Task join error: {e}")))?
}

/// Asynchronously searches for nearest neighbors.
///
/// Wraps `Collection::search()` in `spawn_blocking` for consistency,
/// though search is typically fast enough that this may not be necessary.
///
/// # Arguments
///
/// * `collection` - Arc-wrapped collection instance
/// * `query` - Query vector
/// * `k` - Number of nearest neighbors to return
///
/// # Errors
///
/// Returns an error if the query dimension doesn't match.
pub async fn search_async(
    collection: Arc<Collection>,
    query: Vec<f32>,
    k: usize,
) -> Result<Vec<crate::point::SearchResult>> {
    tokio::task::spawn_blocking(move || collection.search(&query, k))
        .await
        .map_err(|e| Error::Internal(format!("Task join error: {e}")))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DistanceMetric;
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
}
