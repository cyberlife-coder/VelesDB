//! Streaming ingestion pipeline for continuous vector insertion.
//!
//! Provides [`StreamIngester`] which accepts points via a bounded tokio mpsc
//! channel and drains micro-batches into the existing [`Collection::upsert`]
//! pipeline. Backpressure is signaled via [`BackpressureError::BufferFull`]
//! when the channel is at capacity.

#[cfg(feature = "persistence")]
pub mod delta;

#[cfg(feature = "persistence")]
mod ingester;

#[cfg(feature = "persistence")]
pub use delta::merge_with_delta;
#[cfg(feature = "persistence")]
#[allow(unused_imports)]
pub(crate) use ingester::WriteMode;
#[cfg(feature = "persistence")]
pub use ingester::{BackpressureError, StreamIngester, StreamingConfig};
