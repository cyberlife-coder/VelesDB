//! Type-erased collection handle for callers that don't know the collection type.
//!
//! `AnyCollection` wraps the three typed collections in an enum, dispatching
//! common operations via match arms. Zero-cost: no heap allocation, no vtable.
//!
//! # Variant access
//!
//! Three complementary APIs follow the std `Result` / `Option` / `Any` idiom:
//!
//! | Need                              | Method                               | Returns                            |
//! |-----------------------------------|--------------------------------------|------------------------------------|
//! | Check variant                     | [`is_vector`], [`is_graph`], …       | `bool`                             |
//! | Borrow variant (shared)           | [`as_vector`], [`as_graph`], …       | `Option<&T>`                       |
//! | Borrow variant (exclusive)        | [`as_vector_mut`], …                 | `Option<&mut T>`                   |
//! | Consume with recovery on miss     | [`into_vector`], [`into_graph`], …   | `Result<T, Self>`                  |
//!
//! [`is_vector`]: AnyCollection::is_vector
//! [`is_graph`]: AnyCollection::is_graph
//! [`as_vector`]: AnyCollection::as_vector
//! [`as_graph`]: AnyCollection::as_graph
//! [`as_vector_mut`]: AnyCollection::as_vector_mut
//! [`into_vector`]: AnyCollection::into_vector
//! [`into_graph`]: AnyCollection::into_graph

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
///
/// # Examples
///
/// ```rust,no_run
/// use velesdb_core::{AnyCollection, Database};
///
/// let db = Database::open("./data")?;
/// if let Some(any) = db.get_any_collection("docs") {
///     // `config()`, `flush()`, `point_count()`, `name()`, `execute_query_str()`
///     // dispatch across all variants — safe on every kind.
///     println!("{}: {} pts", any.name(), any.point_count());
///
///     // Pattern-match when a variant-specific method is needed.
///     match &any {
///         AnyCollection::Vector(_) => println!("vector collection"),
///         AnyCollection::Graph(_)  => println!("graph collection"),
///         AnyCollection::Metadata(_) => println!("metadata collection"),
///         _ => println!("unknown variant"),
///     }
/// }
/// # Ok::<(), velesdb_core::Error>(())
/// ```
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
    // -------------------------------------------------------------------------
    // Shared operations (dispatch on variant)
    // -------------------------------------------------------------------------

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
    ///
    /// Equivalent to [`is_metadata`](Self::is_metadata) — kept for backward
    /// compatibility with older call sites.
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

    // -------------------------------------------------------------------------
    // Variant discriminants (`is_*`)
    // -------------------------------------------------------------------------

    /// Returns `true` if this collection is the [`Vector`](Self::Vector) variant.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use velesdb_core::{AnyCollection, Database, DistanceMetric};
    ///
    /// let db = Database::open("./data")?;
    /// db.create_collection("docs", 768, DistanceMetric::Cosine)?;
    /// let any = db.get_any_collection("docs").expect("exists");
    /// assert!(any.is_vector());
    /// assert!(!any.is_graph());
    /// # Ok::<(), velesdb_core::Error>(())
    /// ```
    #[must_use]
    pub fn is_vector(&self) -> bool {
        matches!(self, Self::Vector(_))
    }

    /// Returns `true` if this collection is the [`Graph`](Self::Graph) variant.
    #[must_use]
    pub fn is_graph(&self) -> bool {
        matches!(self, Self::Graph(_))
    }

    /// Returns `true` if this collection is the [`Metadata`](Self::Metadata) variant.
    #[must_use]
    pub fn is_metadata(&self) -> bool {
        matches!(self, Self::Metadata(_))
    }

    // -------------------------------------------------------------------------
    // Shared borrows (`as_*`) — zero-cost, return `Option<&T>`
    // -------------------------------------------------------------------------

    /// Returns a shared reference to the inner [`VectorCollection`] if this is
    /// the [`Vector`](Self::Vector) variant, or `None` otherwise.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use velesdb_core::{Database, DistanceMetric};
    ///
    /// let db = Database::open("./data")?;
    /// db.create_collection("docs", 768, DistanceMetric::Cosine)?;
    /// let any = db.get_any_collection("docs").expect("exists");
    /// if let Some(v) = any.as_vector() {
    ///     let _ = v.config().dimension;
    /// }
    /// # Ok::<(), velesdb_core::Error>(())
    /// ```
    #[must_use]
    pub fn as_vector(&self) -> Option<&VectorCollection> {
        match self {
            Self::Vector(c) => Some(c),
            _ => None,
        }
    }

    /// Returns an exclusive reference to the inner [`VectorCollection`] if
    /// this is the [`Vector`](Self::Vector) variant, or `None` otherwise.
    #[must_use]
    pub fn as_vector_mut(&mut self) -> Option<&mut VectorCollection> {
        match self {
            Self::Vector(c) => Some(c),
            _ => None,
        }
    }

    /// Returns a shared reference to the inner [`GraphCollection`] if this is
    /// the [`Graph`](Self::Graph) variant, or `None` otherwise.
    #[must_use]
    pub fn as_graph(&self) -> Option<&GraphCollection> {
        match self {
            Self::Graph(c) => Some(c),
            _ => None,
        }
    }

    /// Returns an exclusive reference to the inner [`GraphCollection`] if
    /// this is the [`Graph`](Self::Graph) variant, or `None` otherwise.
    #[must_use]
    pub fn as_graph_mut(&mut self) -> Option<&mut GraphCollection> {
        match self {
            Self::Graph(c) => Some(c),
            _ => None,
        }
    }

    /// Returns a shared reference to the inner [`MetadataCollection`] if this
    /// is the [`Metadata`](Self::Metadata) variant, or `None` otherwise.
    #[must_use]
    pub fn as_metadata(&self) -> Option<&MetadataCollection> {
        match self {
            Self::Metadata(c) => Some(c),
            _ => None,
        }
    }

    /// Returns an exclusive reference to the inner [`MetadataCollection`] if
    /// this is the [`Metadata`](Self::Metadata) variant, or `None` otherwise.
    #[must_use]
    pub fn as_metadata_mut(&mut self) -> Option<&mut MetadataCollection> {
        match self {
            Self::Metadata(c) => Some(c),
            _ => None,
        }
    }

    // -------------------------------------------------------------------------
    // Consuming conversions (`into_*`) — return `Result<T, Self>` for recovery
    // -------------------------------------------------------------------------

    /// Consumes `self` and returns the inner [`VectorCollection`] if this is
    /// the [`Vector`](Self::Vector) variant.
    ///
    /// On the wrong variant, returns `Err(self)` so callers can recover
    /// ownership — mirroring the std [`Result`] / [`TryFrom`] idiom.
    ///
    /// # Errors
    ///
    /// Returns the original `AnyCollection` unchanged when the variant is
    /// [`Graph`](Self::Graph) or [`Metadata`](Self::Metadata).
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use velesdb_core::{Database, DistanceMetric};
    ///
    /// let db = Database::open("./data")?;
    /// db.create_collection("docs", 768, DistanceMetric::Cosine)?;
    /// let any = db.get_any_collection("docs").expect("exists");
    /// match any.into_vector() {
    ///     Ok(v) => { let _ = v.config().dimension; }
    ///     Err(original) => {
    ///         // wrong variant; `original` still valid
    ///         assert!(!original.is_vector());
    ///     }
    /// }
    /// # Ok::<(), velesdb_core::Error>(())
    /// ```
    // `Err`-variant is `Self` by design — mirrors std `TryFrom` so callers
    // recover ownership on the wrong variant. Box-wrapping would defeat the
    // purpose and forces an allocation on every miss.
    #[allow(clippy::result_large_err)]
    pub fn into_vector(self) -> core::result::Result<VectorCollection, Self> {
        match self {
            Self::Vector(c) => Ok(c),
            other => Err(other),
        }
    }

    /// Consumes `self` and returns the inner [`GraphCollection`] if this is
    /// the [`Graph`](Self::Graph) variant.
    ///
    /// # Errors
    ///
    /// Returns the original `AnyCollection` unchanged when the variant is
    /// [`Vector`](Self::Vector) or [`Metadata`](Self::Metadata).
    #[allow(clippy::result_large_err)]
    pub fn into_graph(self) -> core::result::Result<GraphCollection, Self> {
        match self {
            Self::Graph(c) => Ok(c),
            other => Err(other),
        }
    }

    /// Consumes `self` and returns the inner [`MetadataCollection`] if this
    /// is the [`Metadata`](Self::Metadata) variant.
    ///
    /// # Errors
    ///
    /// Returns the original `AnyCollection` unchanged when the variant is
    /// [`Vector`](Self::Vector) or [`Graph`](Self::Graph).
    #[allow(clippy::result_large_err)]
    pub fn into_metadata(self) -> core::result::Result<MetadataCollection, Self> {
        match self {
            Self::Metadata(c) => Ok(c),
            other => Err(other),
        }
    }

    // -------------------------------------------------------------------------
    // Unchecked cross-cast (escape hatch for SDK bindings)
    // -------------------------------------------------------------------------

    /// Consumes `self` and returns a [`VectorCollection`] regardless of the
    /// underlying variant, re-wrapping the shared inner state.
    ///
    /// For the [`Vector`](Self::Vector) variant this is a straightforward
    /// move. For the [`Graph`](Self::Graph) and [`Metadata`](Self::Metadata)
    /// variants this re-wraps the shared `Arc<Collection>` in the
    /// `VectorCollection` newtype **without changing the underlying runtime
    /// type** — downstream code that invokes vector-specific methods on the
    /// result (for example [`search`](VectorCollection::search),
    /// [`upsert`](VectorCollection::upsert),
    /// `config().dimension > 0`) may therefore return empty results or
    /// misleading state.
    ///
    /// This method exists to support the Python / Mobile / Tauri SDK bindings
    /// that expose a single `Collection` type to users and only invoke the
    /// shared surface (`config`, `flush`, `diagnostics`, `point_count`,
    /// `execute_query_str`) on the result.
    ///
    /// # Safety
    ///
    /// Calling vector-specific methods on a `VectorCollection` obtained from
    /// a `Graph` or `Metadata` variant is **not** memory-unsafe, but the
    /// result is logically unsound: the underlying storage does not hold a
    /// homogeneous vector index, and the returned search results are either
    /// empty or reflect internal state that was not intended for public
    /// consumption.
    ///
    /// Callers must either:
    ///
    /// * branch on [`is_vector`](Self::is_vector) first and only invoke
    ///   vector-specific methods on the `Vector` variant, or
    /// * restrict themselves to the methods that all three collection
    ///   kinds share (`config`, `flush`, `diagnostics`, `name`,
    ///   `point_count`, `execute_query_str`).
    ///
    /// Prefer the safe [`into_vector`](Self::into_vector) (variant-checked,
    /// returns `Result`) when the caller can branch. A proper type-safe
    /// refactor that eliminates this method entirely is tracked under the
    /// post-seed EPIC documented in `docs/ARCHITECTURE.md` (finding F2.2 of
    /// the pre-seed audit).
    ///
    /// # Violation of invariants
    ///
    /// The `unsafe` marker flags the caller contract (only invoke the
    /// shared surface on non-`Vector` variants) even though violating it
    /// does not cause undefined behaviour.
    #[must_use]
    pub unsafe fn into_vector_unchecked(self) -> VectorCollection {
        match self {
            Self::Vector(c) => c,
            Self::Graph(c) => VectorCollection { inner: c.inner },
            Self::Metadata(c) => VectorCollection { inner: c.inner },
        }
    }
}
