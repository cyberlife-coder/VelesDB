#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::float_cmp,
    clippy::approx_constant
)]
//! Async wrappers for blocking storage operations.
//!
//! EPIC-034/US-001: Provides `spawn_blocking` wrappers for I/O-intensive
//! storage operations to avoid blocking the async executor.
//!
//! # Why `spawn_blocking`?
//!
//! Memory-mapped file operations (mmap resize, flush, compaction) perform
//! blocking syscalls that can stall the async runtime. This module wraps
//! these operations to run on Tokio's blocking thread pool.
//!
//! # Usage
//!
//! ```rust,ignore
//! use velesdb_core::storage::{MmapStorage, async_ops};
//!
//! async fn bulk_import(storage: Arc<RwLock<MmapStorage>>) {
//!     // Pre-allocate in blocking thread
//!     async_ops::reserve_capacity_async(storage.clone(), 1_000_000).await?;
//!
//!     // Then insert vectors...
//! }
//! ```

use parking_lot::RwLock;
use std::io;
use std::sync::Arc;

use super::traits::VectorStorage;
use super::MmapStorage;

/// Asynchronously reserves storage capacity for a known number of vectors.
///
/// Wraps `MmapStorage::reserve_capacity()` in `spawn_blocking` to avoid
/// blocking the async executor during file resize operations.
///
/// # Arguments
///
/// * `storage` - Arc-wrapped storage instance
/// * `vector_count` - Expected number of vectors to store
///
/// # Errors
///
/// Returns an error if file operations fail or if the blocking task panics.
pub async fn reserve_capacity_async(
    storage: Arc<RwLock<MmapStorage>>,
    vector_count: usize,
) -> io::Result<()> {
    tokio::task::spawn_blocking(move || {
        let mut guard = storage.write();
        guard.reserve_capacity(vector_count)
    })
    .await
    .map_err(|e| io::Error::other(format!("Task join error: {e}")))?
}

/// Asynchronously compacts the storage by rewriting only active vectors.
///
/// Wraps `MmapStorage::compact()` in `spawn_blocking` to avoid blocking
/// the async executor during the potentially long compaction operation.
///
/// # Returns
///
/// The number of bytes reclaimed.
///
/// # Errors
///
/// Returns an error if file operations fail or if the blocking task panics.
pub async fn compact_async(storage: Arc<RwLock<MmapStorage>>) -> io::Result<usize> {
    tokio::task::spawn_blocking(move || {
        let mut guard = storage.write();
        guard.compact()
    })
    .await
    .map_err(|e| io::Error::other(format!("Task join error: {e}")))?
}

/// Asynchronously flushes the storage to disk.
///
/// Wraps `MmapStorage::flush()` in `spawn_blocking` to avoid blocking
/// the async executor during disk sync operations.
///
/// # Errors
///
/// Returns an error if file operations fail or if the blocking task panics.
pub async fn flush_async(storage: Arc<RwLock<MmapStorage>>) -> io::Result<()> {
    tokio::task::spawn_blocking(move || {
        let mut guard = storage.write();
        guard.flush()
    })
    .await
    .map_err(|e| io::Error::other(format!("Task join error: {e}")))?
}

/// Asynchronously stores a batch of vectors.
///
/// Wraps bulk insertion in `spawn_blocking` for large batches that would
/// otherwise block the async executor.
///
/// # Arguments
///
/// * `storage` - Arc-wrapped storage instance
/// * `vectors` - Vector of (id, `vector_data`) pairs
///
/// # Errors
///
/// Returns an error if any store operation fails.
pub async fn store_batch_async(
    storage: Arc<RwLock<MmapStorage>>,
    vectors: Vec<(u64, Vec<f32>)>,
) -> io::Result<usize> {
    tokio::task::spawn_blocking(move || {
        let mut guard = storage.write();
        let mut count = 0;
        for (id, vector) in vectors {
            guard.store(id, &vector)?;
            count += 1;
        }
        Ok(count)
    })
    .await
    .map_err(|e| io::Error::other(format!("Task join error: {e}")))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_reserve_capacity_async() {
        let dir = TempDir::new().unwrap();
        let storage = Arc::new(RwLock::new(MmapStorage::new(dir.path(), 128).unwrap()));

        let result = reserve_capacity_async(storage, 1000).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_store_batch_async() {
        let dir = TempDir::new().unwrap();
        let storage = Arc::new(RwLock::new(MmapStorage::new(dir.path(), 4).unwrap()));

        let vectors: Vec<(u64, Vec<f32>)> = (0..100).map(|i| (i, vec![i as f32; 4])).collect();

        let result = store_batch_async(storage.clone(), vectors).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 100);
    }

    #[tokio::test]
    async fn test_flush_async() {
        let dir = TempDir::new().unwrap();
        let storage = Arc::new(RwLock::new(MmapStorage::new(dir.path(), 128).unwrap()));

        let result = flush_async(storage).await;
        assert!(result.is_ok());
    }
}
