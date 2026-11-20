//! Collection health diagnostics for embedded SDK usage.
//!
//! Provides [`CollectionDiagnostics`] and [`IndexHealth`] to let developers
//! inspect a collection's readiness without relying on the REST server.

use super::types::Collection;
use super::{GraphCollection, MetadataCollection, VectorCollection};

/// Health status of a collection's search index.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum IndexHealth {
    /// Index is populated and ready for search.
    Healthy,
    /// Index needs to be rebuilt (e.g., after corruption or schema change).
    NeedsRebuild(String),
    /// Index is empty (no data ingested yet).
    Empty,
}

/// Diagnostic snapshot of a collection's state.
///
/// Returned by [`VectorCollection::diagnostics()`],
/// [`GraphCollection::diagnostics()`],
/// [`MetadataCollection::diagnostics()`],
/// and [`Database::collection_diagnostics()`](crate::Database::collection_diagnostics).
#[derive(Debug, Clone)]
pub struct CollectionDiagnostics {
    /// Whether the collection contains at least one vector: a metadata
    /// collection's points count, a graph's nodes without an embedding do
    /// not.
    pub has_vectors: bool,
    /// Whether the collection is ready to serve search queries.
    pub search_ready: bool,
    /// Whether a valid dimension is configured (> 0 for vector collections).
    pub dimension_configured: bool,
    /// Total number of points in the collection; for a graph, every stored
    /// node ([`GraphCollection::len`]).
    pub point_count: usize,
    /// Health status of the primary search index.
    pub index_health: IndexHealth,
}

impl CollectionDiagnostics {
    /// Builds diagnostics from a `Collection` instance: `point_count` is its
    /// size ([`Collection::len`]) — a graph's payload-only nodes included —
    /// while what is about the search index reads the vectors alone.
    pub(crate) fn from_collection(coll: &Collection) -> Self {
        let point_count = coll.len();
        let has_vectors = coll.vector_count() > 0;
        let dimension_configured = coll.config().dimension > 0;
        let search_ready = has_vectors && dimension_configured;
        let index_health = if has_vectors {
            IndexHealth::Healthy
        } else {
            IndexHealth::Empty
        };

        Self {
            has_vectors,
            search_ready,
            dimension_configured,
            point_count,
            index_health,
        }
    }

    /// Builds diagnostics for a metadata-only collection.
    pub(crate) fn from_metadata(coll: &Collection) -> Self {
        let point_count = coll.len();

        Self {
            has_vectors: point_count > 0,
            search_ready: false,
            dimension_configured: false,
            point_count,
            index_health: if point_count == 0 {
                IndexHealth::Empty
            } else {
                IndexHealth::Healthy
            },
        }
    }
}

impl VectorCollection {
    /// Returns diagnostic information about this collection.
    #[must_use]
    pub fn diagnostics(&self) -> CollectionDiagnostics {
        CollectionDiagnostics::from_collection(&self.inner)
    }
}

impl GraphCollection {
    /// Returns diagnostic information about this collection: `point_count`
    /// is [`len`](Self::len), every stored node; `has_vectors`,
    /// `search_ready` and `index_health` read the node embeddings.
    #[must_use]
    pub fn diagnostics(&self) -> CollectionDiagnostics {
        CollectionDiagnostics::from_collection(&self.inner)
    }
}

impl MetadataCollection {
    /// Returns diagnostic information about this collection.
    #[must_use]
    pub fn diagnostics(&self) -> CollectionDiagnostics {
        CollectionDiagnostics::from_metadata(&self.inner)
    }
}
