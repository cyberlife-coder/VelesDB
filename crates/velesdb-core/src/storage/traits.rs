//! Storage traits for vectors and payloads.
//!
//! This module defines the core storage abstractions used by `VelesDB`.

use std::io;

/// Trait defining storage operations for vectors.
pub trait VectorStorage: Send + Sync {
    /// Stores a vector with the given ID.
    ///
    /// Durability note: implementations may buffer writes. Call [`flush`](Self::flush)
    /// to obtain an explicit durability barrier.
    ///
    /// # Errors
    ///
    /// Returns an error if the write operation fails.
    fn store(&mut self, id: u64, vector: &[f32]) -> io::Result<()>;

    /// Stores multiple vectors in a single batch operation.
    ///
    /// This is optimized for bulk imports:
    /// - Single WAL write for the entire batch
    /// - Contiguous memory writes
    ///
    /// Durability note: batch writes may be buffered. Call [`flush`](Self::flush)
    /// after `store_batch` to force persistence guarantees.
    ///
    /// # Errors
    ///
    /// Returns an error if the write operation fails.
    fn store_batch(&mut self, vectors: &[(u64, &[f32])]) -> io::Result<usize>;

    /// Retrieves a vector by ID.
    ///
    /// # Errors
    ///
    /// Returns an error if the read operation fails.
    fn retrieve(&self, id: u64) -> io::Result<Option<Vec<f32>>>;

    /// Deletes a vector by ID.
    ///
    /// # Errors
    ///
    /// Returns an error if the delete operation fails.
    fn delete(&mut self, id: u64) -> io::Result<()>;

    /// Flushes pending writes to disk.
    ///
    /// This is the explicit durability barrier. Callers that require
    /// deterministic crash consistency must call this method.
    ///
    /// # Errors
    ///
    /// Returns an error if the flush operation fails.
    fn flush(&mut self) -> io::Result<()>;

    /// Returns the number of vectors stored.
    fn len(&self) -> usize;

    /// Returns true if the storage is empty.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns all stored IDs.
    fn ids(&self) -> Vec<u64>;
}

/// Trait defining storage operations for metadata payloads.
pub trait PayloadStorage: Send + Sync {
    /// Stores a payload with the given ID.
    ///
    /// Durability note: implementations may buffer writes. Call [`flush`](Self::flush)
    /// to obtain an explicit durability barrier.
    ///
    /// # Errors
    ///
    /// Returns an error if the write operation fails.
    fn store(&mut self, id: u64, payload: &serde_json::Value) -> io::Result<()>;

    /// Retrieves a payload by ID.
    ///
    /// # Errors
    ///
    /// Returns an error if the read operation fails.
    fn retrieve(&self, id: u64) -> io::Result<Option<serde_json::Value>>;

    /// Deletes a payload by ID.
    ///
    /// # Errors
    ///
    /// Returns an error if the delete operation fails.
    fn delete(&mut self, id: u64) -> io::Result<()>;

    /// Flushes pending writes to disk.
    ///
    /// This is the explicit durability barrier. Callers that require
    /// deterministic crash consistency must call this method.
    ///
    /// # Errors
    ///
    /// Returns an error if the flush operation fails.
    fn flush(&mut self) -> io::Result<()>;

    /// Returns all stored IDs.
    fn ids(&self) -> Vec<u64>;
}
