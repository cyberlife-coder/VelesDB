//! Storage backends for persistent vector storage.
//!
//! This module contains memory-mapped file storage implementation for vectors
//! and log-structured storage for metadata payloads.
//!
//! # Module Structure
//!
//! - [`traits`]: Storage traits (`VectorStorage`, `PayloadStorage`)
//! - [`mmap`]: Memory-mapped vector storage (`MmapStorage`)
//! - [`log_payload`]: Log-structured payload storage (`LogPayloadStorage`)
//! - [`guard`]: Zero-copy vector slice guard (`VectorSliceGuard`)
//! - [`metrics`]: Storage operation metrics (P0 audit - latency monitoring)

mod guard;
mod histogram;
mod log_payload;
pub mod metrics;
mod mmap;
mod traits;

#[cfg(test)]
mod histogram_tests;
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
