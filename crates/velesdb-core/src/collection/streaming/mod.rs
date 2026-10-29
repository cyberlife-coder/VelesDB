//! Streaming ingestion pipeline for continuous vector insertion.
//!
//! Provides [`StreamIngester`] which accepts points via a bounded tokio mpsc
//! channel and drains micro-batches into the existing `Collection::upsert`
//! pipeline. Backpressure is signaled via `BackpressureError::BufferFull`
//! when the channel is at capacity.

pub mod async_index_builder;

#[cfg(feature = "persistence")]
pub mod delta;

#[cfg(feature = "persistence")]
mod delta_merge;

#[cfg(feature = "persistence")]
pub mod deferred;

#[cfg(feature = "persistence")]
mod ingester;

pub use async_index_builder::{AsyncIndexBuilder, AsyncIndexBuilderConfig};

#[cfg(feature = "persistence")]
pub use deferred::{DeferredIndexer, DeferredIndexerConfig};
#[cfg(feature = "persistence")]
pub use delta_merge::{merge_with_delta, merge_with_delta_scored};
#[cfg(feature = "persistence")]
pub use ingester::{BackpressureError, StreamIngester, StreamingConfig};

/// A streaming setting persisted in `config.json` but not yet in force.
///
/// [`VectorCollection::apply_advanced_config`] installs a deferred indexer or
/// an async index builder at once on a collection that runs none. It never
/// replaces or removes one that is live: writes may be buffered in it, and no
/// lock spans a write's use of it, so only an open — which builds the
/// components from the persisted config before anyone holds them — can change
/// it safely. Such a change is persisted, and reported by
/// [`VectorCollection::pending_reopen`] until the collection is next opened.
///
/// [`VectorCollection::apply_advanced_config`]: crate::VectorCollection::apply_advanced_config
/// [`VectorCollection::pending_reopen`]: crate::VectorCollection::pending_reopen
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PendingReopen {
    /// `deferred_indexing` differs from the deferred indexer in use.
    DeferredIndexing,
    /// `async_index_builder` differs from the async index builder in use.
    AsyncIndexBuilder,
}

impl PendingReopen {
    /// The `CollectionConfig` field (and REST / binding key) this names.
    #[must_use]
    pub const fn field_name(self) -> &'static str {
        match self {
            Self::DeferredIndexing => "deferred_indexing",
            Self::AsyncIndexBuilder => "async_index_builder",
        }
    }
}

#[cfg(test)]
mod async_index_builder_tests;
#[cfg(all(test, feature = "persistence"))]
mod delta_tests;
#[cfg(all(test, feature = "persistence"))]
mod ingester_tests;
