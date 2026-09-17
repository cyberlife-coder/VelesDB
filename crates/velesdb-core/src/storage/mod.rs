//! Storage backends for persistent vector storage.
//!
//! This module contains memory-mapped file storage implementation for vectors
//! and log-structured storage for metadata payloads.
//!
//! # Public Types
//!
//! - [`VectorStorage`], [`PayloadStorage`]: Storage traits
//! - [`MmapStorage`]: Memory-mapped vector storage
//! - [`LogPayloadStorage`]: Log-structured payload storage
//! - [`VectorSliceGuard`]: Zero-copy vector slice guard
//! - [`metrics`]: Storage operation metrics (P0 audit - latency monitoring)
//! - [`async_ops`]: Async wrappers for blocking I/O (EPIC-034/US-001)
//! - [`wal_cursor`]: Shippable WAL cursor — additive, read-only API over the
//!   existing WAL framing for replication consumers (no on-disk format change)
#![allow(clippy::doc_markdown)] // Storage docs include API and platform identifiers.

pub mod async_ops;
pub(crate) mod atomic_write;
mod compaction;
mod guard;
mod histogram;
mod log_payload;
mod log_payload_io;
pub mod metrics;
mod mmap;
mod mmap_capacity;
mod sharded_index;
#[cfg(test)]
mod sharded_index_tests;
pub(crate) mod snapshot;
mod traits;
pub mod vector_bytes;
#[cfg(test)]
mod vector_bytes_tests;
pub mod wal_cursor;
#[cfg(test)]
mod wal_cursor_legacy_tests;
#[cfg(test)]
mod wal_cursor_property_tests;
pub mod wal_cursor_reader;
mod wal_entry;
#[cfg(test)]
mod wal_retention_property_tests;

#[cfg(test)]
mod compaction_tests;
#[cfg(test)]
mod deferred_index_tests;
#[cfg(test)]
mod guard_tests;
#[cfg(test)]
mod histogram_tests;
#[cfg(test)]
mod idx_persistence_tests;
#[cfg(test)]
mod log_payload_tests;
#[cfg(test)]
mod loom_tests;
#[cfg(test)]
mod metrics_tests;
#[cfg(test)]
mod mmap_durability_tests;
#[cfg(test)]
mod snapshot_tests;
#[cfg(test)]
mod storage_reliability_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod wal_recovery_tests;

// Re-export public types
pub use guard::VectorSliceGuard;
pub use log_payload::{DurabilityMode, LogPayloadStorage};
pub use metrics::{LatencyStats, StorageMetrics};
pub use mmap::MmapStorage;
pub use traits::{PayloadStorage, VectorStorage};
pub use wal_cursor::{WalConsumerId, WalCursor, WalPosition, WalRecord, WalWatermarkRegistry};
pub use wal_cursor_reader::LogWalCursor;

/// Parses payload-snapshot bytes with the parser [`LogPayloadStorage`] runs
/// when it opens, and discards the parsed index.
///
/// This is the entry `fuzz/fuzz_targets/fuzz_snapshot_parser.rs` drives. It is
/// compiled only under `--cfg fuzzing`, which cargo-fuzz sets, and in this
/// crate's unit tests, where `snapshot_tests` calls it: every PR builds and
/// runs the entry itself. Its `fuzzing` gate and the fuzz target's call are
/// not checked by any PR, because no PR job builds `fuzz/` (#2311). No
/// ordinary build contains it, and it is not part of the API.
///
/// # Errors
///
/// Returns `InvalidData` if `data` is not a well-formed snapshot.
#[cfg(any(fuzzing, test))]
#[doc(hidden)]
pub fn parse_payload_snapshot(data: &[u8]) -> std::io::Result<()> {
    snapshot::parse_snapshot(data).map(drop)
}
