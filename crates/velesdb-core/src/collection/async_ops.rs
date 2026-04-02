//! Async wrappers for blocking Collection operations.
//!
//! EPIC-034/US-005: Provides async bulk insert API using `spawn_blocking`
//! to avoid blocking the async executor during I/O-intensive operations.
//!
//! # Why `spawn_blocking`?
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
//! use velesdb_core::{VectorCollection, Point};
//! use velesdb_core::collection::async_ops;
//! use std::sync::Arc;
//!
//! async fn bulk_import(collection: Arc<VectorCollection>) -> Result<usize, Error> {
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
/// # Performance — Deferred WAL fsync
///
/// Intermediate batches use [`Collection::upsert_bulk_deferred_sync`] which
/// skips `sync_all()` after each batch, replacing it with a buffer-only
/// `flush()`. This eliminates the 1-5ms per-batch fsync overhead on Windows.
/// For 1M vectors at 5K/batch, this saves 200 batches x 2 fsyncs x 1-5ms
/// = 400-2000ms of pure I/O overhead.
///
/// A single `Collection::flush()` after the final batch ensures full
/// durability. In case of a crash before that final flush, data since the
/// last synced point is lost but the database remains consistent (WAL
/// replay recovers to the last fsync boundary).
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
    let num_chunks = total.div_ceil(chunk_size);

    for (chunk_idx, chunk) in points.chunks(chunk_size).enumerate() {
        let chunk_vec: Vec<Point> = chunk.to_vec();
        let coll = Arc::clone(&collection);
        let is_last = chunk_idx + 1 == num_chunks;

        // H3: Intermediate batches skip fsync (deferred sync).
        // The final batch uses the standard path with full fsync.
        let count = if is_last {
            tokio::task::spawn_blocking(move || coll.upsert_bulk(&chunk_vec))
                .await
                .map_err(|e| Error::Internal(format!("Task join error: {e}")))?
        } else {
            tokio::task::spawn_blocking(move || coll.upsert_bulk_deferred_sync(&chunk_vec))
                .await
                .map_err(|e| Error::Internal(format!("Task join error: {e}")))?
        };

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

    // H3: Final durability barrier — ensures all deferred WAL data is fsynced.
    // Even though the last batch used upsert_bulk (with fsync), an explicit
    // flush guarantees the entire pipeline is durable.
    let coll = Arc::clone(&collection);
    tokio::task::spawn_blocking(move || coll.flush())
        .await
        .map_err(|e| Error::Internal(format!("Task join error: {e}")))?
        .map_err(|e| Error::Internal(format!("Final flush error: {e}")))?;

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

// Tests moved to async_ops_tests.rs per project rules
