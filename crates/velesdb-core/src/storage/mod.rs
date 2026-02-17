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
#![allow(clippy::doc_markdown)] // Storage docs include API and platform identifiers.

pub mod async_ops;
mod compaction;
mod guard;
mod histogram;
mod log_payload;
pub mod metrics;
mod mmap;
mod sharded_index;
#[cfg(test)]
mod sharded_index_tests;
mod traits;
pub mod vector_bytes;
#[cfg(test)]
mod vector_bytes_tests;

#[cfg(test)]
mod histogram_tests;
#[cfg(test)]
mod log_payload_tests;
#[cfg(test)]
mod loom_tests;
#[cfg(test)]
mod metrics_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod wal_recovery_tests;

// Re-export public types
pub use guard::VectorSliceGuard;
pub use log_payload::LogPayloadStorage;
pub use metrics::{LatencyStats, StorageMetrics};
pub use mmap::MmapStorage;
pub use traits::{PayloadStorage, VectorStorage};
