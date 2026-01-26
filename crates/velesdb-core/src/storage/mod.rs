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

mod compaction;
mod guard;
mod histogram;
mod log_payload;
pub mod metrics;
mod mmap;
mod sharded_index;
mod traits;
mod vector_bytes;
#[cfg(test)]
mod vector_bytes_tests;

#[cfg(test)]
mod histogram_tests;
#[cfg(test)]
mod log_payload_tests;
#[cfg(test)]
mod metrics_tests;
#[cfg(test)]
mod tests;

// Re-export public types
pub use guard::VectorSliceGuard;
pub use log_payload::LogPayloadStorage;
pub use metrics::{LatencyStats, StorageMetrics};
pub use mmap::MmapStorage;
pub use traits::{PayloadStorage, VectorStorage};
