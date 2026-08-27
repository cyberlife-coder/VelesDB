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
//! # Why spawn_blocking?
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

/// Asynchronously stores a batch of vectors, paying one durability barrier
/// for the whole batch.
///
/// Wraps bulk insertion in `spawn_blocking` for large batches that would
/// otherwise block the async executor.
///
/// This used to loop over [`VectorStorage::store`], which under
/// [`DurabilityMode::Fsync`](super::DurabilityMode::Fsync) issues its own
/// `flush` + `sync_all` per vector — so a 10 000-vector batch paid 10 000
/// fsyncs, every one of them while holding the storage write lock, which is
/// the opposite of what the name promises. [`VectorStorage::store_batch`]
/// coalesces the WAL entries into a single grouped write and deliberately
/// leaves the barrier to the caller; the explicit `flush` afterwards is that
/// barrier.
///
/// The durability guarantee on return is unchanged — every entry is on disk
/// when this resolves under `Fsync` — because `flush` dispatches on the same
/// mode the per-vector path consulted. It is in fact marginally stronger: the
/// mmap is flushed too, which the per-vector loop never did.
///
/// # Arguments
///
/// * `storage` - Arc-wrapped storage instance
/// * `vectors` - Vector of (id, vector_data) pairs
///
/// # Errors
///
/// Returns an error if any dimension is wrong, if the write fails, or if the
/// durability barrier fails. Dimensions are validated for the whole batch
/// before anything is written, so a malformed entry rejects the batch instead
/// of leaving the prefix before it committed.
pub async fn store_batch_async(
    storage: Arc<RwLock<MmapStorage>>,
    vectors: Vec<(u64, Vec<f32>)>,
) -> io::Result<usize> {
    tokio::task::spawn_blocking(move || {
        let borrowed: Vec<(u64, &[f32])> = vectors
            .iter()
            .map(|(id, vector)| (*id, vector.as_slice()))
            .collect();

        let mut guard = storage.write();
        let count = guard.store_batch(&borrowed)?;
        guard.flush()?;
        Ok(count)
    })
    .await
    .map_err(|e| io::Error::other(format!("Task join error: {e}")))?
}

#[cfg(test)]
#[path = "async_ops_tests.rs"]
mod tests;
