//! Type-erased collection handle for callers that don't know the collection type.
//!
//! `AnyCollection` wraps the three typed collections in an enum, dispatching
//! common operations via match arms. Zero-cost: no heap allocation, no vtable.

use std::collections::HashMap;

use crate::collection::graph_collection::GraphCollection;
use crate::collection::metadata_collection::MetadataCollection;
use crate::collection::types::CollectionConfig;
use crate::collection::vector_collection::VectorCollection;
use crate::error::Result;
use crate::point::SearchResult;

/// Type-erased collection handle for callers that don't know the collection type.
///
/// Dispatches common operations to the inner typed collection via enum match.
/// Zero-cost: no heap allocation, no vtable — just a match arm per variant.
#[derive(Clone)]
#[non_exhaustive]
pub enum AnyCollection {
    /// A vector collection (HNSW + payload + full-text).
    Vector(VectorCollection),
    /// A graph collection (edges + optional node embeddings).
    Graph(GraphCollection),
    /// A metadata-only collection (payload, no vectors).
    Metadata(MetadataCollection),
}

impl AnyCollection {
    /// Returns the collection configuration.
    #[must_use]
    pub fn config(&self) -> CollectionConfig {
        match self {
            Self::Vector(c) => c.config(),
            Self::Graph(c) => c.inner.config(),
            Self::Metadata(c) => c.inner.config(),
        }
    }

    /// Flushes all state to disk.
    ///
    /// # Errors
    ///
    /// Returns an error if any flush operation fails.
    pub fn flush(&self) -> Result<()> {
        match self {
            Self::Vector(c) => c.flush(),
            Self::Graph(c) => c.flush(),
            Self::Metadata(c) => c.flush(),
        }
    }

    /// Returns the number of points in the collection.
    #[must_use]
    pub fn point_count(&self) -> usize {
        self.config().point_count
    }

    /// Returns `true` if the collection contains no points.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        match self {
            Self::Vector(c) => c.inner.is_empty(),
            Self::Graph(c) => c.is_empty(),
            Self::Metadata(c) => c.is_empty(),
        }
    }

    /// Returns `true` if this is a metadata-only collection.
    #[must_use]
    pub fn is_metadata_only(&self) -> bool {
        matches!(self, Self::Metadata(_))
    }

    /// Returns the collection name.
    #[must_use]
    pub fn name(&self) -> String {
        self.config().name
    }

    /// Executes a raw VelesQL string, parsing it before execution.
    ///
    /// # Errors
    ///
    /// Returns an error if parsing or execution fails.
    pub fn execute_query_str(
        &self,
        sql: &str,
        params: &HashMap<String, serde_json::Value>,
    ) -> Result<Vec<SearchResult>> {
        match self {
            Self::Vector(c) => c.execute_query_str(sql, params),
            Self::Graph(c) => c.execute_query_str(sql, params),
            Self::Metadata(c) => c.execute_query_str(sql, params),
        }
    }

    /// Executes an aggregation query (GROUP BY / COUNT / SUM / AVG / MIN / MAX).
    ///
    /// # Errors
    ///
    /// Returns an error if the query is invalid or aggregation computation fails.
    pub fn execute_aggregate(
        &self,
        query: &crate::velesql::Query,
        params: &HashMap<String, serde_json::Value>,
    ) -> Result<serde_json::Value> {
        match self {
            Self::Vector(c) => c.execute_aggregate(query, params),
            Self::Graph(c) => c.inner.execute_aggregate(query, params),
            Self::Metadata(c) => c.inner.execute_aggregate(query, params),
        }
    }

    /// Returns collection diagnostics.
    #[must_use]
    pub fn diagnostics(&self) -> crate::collection::CollectionDiagnostics {
        match self {
            Self::Vector(c) => c.diagnostics(),
            Self::Graph(c) => c.diagnostics(),
            Self::Metadata(c) => c.diagnostics(),
        }
    }

    /// Consumes this `AnyCollection` and returns a `VectorCollection`
    /// wrapping the same inner state.
    ///
    /// For the `Vector` variant this is a straightforward move. For
    /// the `Graph` and `Metadata` variants this function re-wraps the
    /// inner `Arc<Collection>` in the `VectorCollection` newtype
    /// without changing the underlying runtime type — downstream code
    /// that then invokes vector-specific methods on the result (for
    /// example `search`, `upsert`, `config().dimension > 0`) may
    /// therefore hit an error or nonsensical state at runtime. The
    /// method is still useful for SDK bindings (Python, Mobile, Tauri)
    /// that expose a single `Collection` type to users and only call
    /// shared methods (config, flush, diagnostics, point_count) on
    /// the result.
    ///
    /// # Safety
    ///
    /// Calling vector-specific methods on a `VectorCollection` that
    /// was produced from a `Graph` or `Metadata` variant is **not**
    /// memory-unsafe, but the result is logically unsound: the
    /// underlying storage does not hold a homogeneous vector index,
    /// and the returned `SearchResult` objects are either empty or
    /// reflect internal state that was not intended for public
    /// consumption. Callers must either:
    ///
    /// * branch on [`AnyCollection::is_vector`] first and only
    ///   invoke vector-specific methods on the `Vector` variant, or
    /// * restrict themselves to the methods that all three collection
    ///   kinds share (`config`, `flush`, `diagnostics`, `name`,
    ///   `point_count`).
    ///
    /// This method is retained to support the existing SDK surface
    /// but is flagged as `unchecked` to make the caller obligation
    /// explicit. A proper type-safe refactor is tracked under the
    /// post-seed EPIC documented in `docs/ARCHITECTURE.md` (finding
    /// F2.2 of the pre-seed audit).
    #[must_use]
    pub fn as_vector_collection_unchecked(self) -> VectorCollection {
        match self {
            Self::Vector(c) => c,
            Self::Graph(c) => VectorCollection { inner: c.inner },
            Self::Metadata(c) => VectorCollection { inner: c.inner },
        }
    }

    /// Deprecated alias for [`Self::as_vector_collection_unchecked`].
    ///
    /// The old name is retained so external consumers do not break at
    /// compile time; it emits a deprecation warning that points at
    /// the safer replacement. Internal call sites have been migrated.
    #[must_use]
    #[deprecated(
        since = "1.13.0",
        note = "Renamed to `as_vector_collection_unchecked`. See the rustdoc for the unchecked-cast contract and finding F2.2 in docs/ARCHITECTURE.md."
    )]
    pub fn into_vector_collection(self) -> VectorCollection {
        self.as_vector_collection_unchecked()
    }

    /// Returns `true` if this collection is a `Vector` variant.
    ///
    /// Intended for callers that use
    /// [`Self::as_vector_collection_unchecked`] and need to branch on
    /// the underlying kind before invoking vector-specific methods.
    #[must_use]
    pub fn is_vector(&self) -> bool {
        matches!(self, Self::Vector(_))
    }
}
