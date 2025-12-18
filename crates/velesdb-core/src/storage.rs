//! Storage backends for persistent vector storage.
//!
//! This module will contain memory-mapped file storage and other backends.

// TODO: Implement mmap-based storage using memmap2
// TODO: Implement WAL (Write-Ahead Log) for durability
// TODO: Implement compaction for space reclamation

/// Placeholder for storage trait.
pub trait VectorStorage: Send + Sync {
    /// Stores a vector with the given ID.
    ///
    /// # Errors
    ///
    /// Returns an I/O error if the write operation fails.
    fn store(&mut self, id: u64, vector: &[f32]) -> std::io::Result<()>;

    /// Retrieves a vector by ID.
    ///
    /// # Errors
    ///
    /// Returns an I/O error if the read operation fails.
    fn retrieve(&self, id: u64) -> std::io::Result<Option<Vec<f32>>>;

    /// Deletes a vector by ID.
    ///
    /// # Errors
    ///
    /// Returns an I/O error if the delete operation fails.
    fn delete(&mut self, id: u64) -> std::io::Result<()>;

    /// Flushes pending writes to disk.
    ///
    /// # Errors
    ///
    /// Returns an I/O error if the flush operation fails.
    fn flush(&mut self) -> std::io::Result<()>;
}
