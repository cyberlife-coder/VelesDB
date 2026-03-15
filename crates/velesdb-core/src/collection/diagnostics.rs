//! Collection health diagnostics for embedded SDK usage.
//!
//! Provides [`CollectionDiagnostics`] and [`IndexHealth`] to let developers
//! inspect a collection's readiness without relying on the REST server.

use super::types::Collection;
use super::{GraphCollection, MetadataCollection, VectorCollection};

/// Health status of a collection's search index.
#[derive(Debug, Clone, PartialEq, Eq)]
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
    /// Whether the collection contains at least one vector/point.
    pub has_vectors: bool,
    /// Whether the collection is ready to serve search queries.
    pub search_ready: bool,
    /// Whether a valid dimension is configured (> 0 for vector collections).
    pub dimension_configured: bool,
    /// Total number of points in the collection.
    pub point_count: usize,
    /// Health status of the primary search index.
    pub index_health: IndexHealth,
}

impl CollectionDiagnostics {
    /// Builds diagnostics from a `Collection` instance.
    pub(crate) fn from_collection(coll: &Collection) -> Self {
        let config = coll.config();
        let point_count = config.point_count;
        let has_vectors = point_count > 0;
        let dimension_configured = config.dimension > 0;
        let search_ready = has_vectors && dimension_configured;
        let index_health = if point_count == 0 {
            IndexHealth::Empty
        } else {
            IndexHealth::Healthy
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
    /// Returns diagnostic information about this collection.
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
