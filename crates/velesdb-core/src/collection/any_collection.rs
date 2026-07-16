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

    /// The shared inner [`Collection`](crate::collection::types::Collection)
    /// backing every variant. All three newtypes wrap the same `Collection`, so
    /// operations that are identical across kinds dispatch through this one
    /// accessor instead of a per-variant match — removing the ad-hoc mix of
    /// `c.method()` / `c.inner.method()` forwarding that the audit flagged (P2.6).
    #[inline]
    fn inner(&self) -> &crate::collection::types::Collection {
        match self {
            Self::Vector(c) => &c.inner,
            Self::Graph(c) => &c.inner,
            Self::Metadata(c) => &c.inner,
        }
    }

    /// Returns the collection configuration.
    #[must_use]
    pub fn config(&self) -> CollectionConfig {
        self.inner().config()
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
        self.inner().is_empty()
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
        self.inner().execute_aggregate(query, params)
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
    // Graph edge operations (shared across all collection types)
    // -------------------------------------------------------------------------

    /// Adds a graph edge.
    ///
    /// # Errors
    ///
    /// Returns an error if the edge cannot be stored.
    pub fn add_edge(&self, edge: crate::collection::graph::GraphEdge) -> Result<()> {
        self.inner().add_edge(edge)
    }

    /// Removes a graph edge by ID. Returns `true` if the edge existed.
    #[must_use]
    pub fn remove_edge(&self, edge_id: u64) -> bool {
        self.inner().remove_edge(edge_id)
    }

    /// Returns outgoing edges from a node.
    #[must_use]
    pub fn get_outgoing_edges(&self, node_id: u64) -> Vec<crate::collection::graph::GraphEdge> {
        self.inner().get_outgoing_edges(node_id)
    }

    /// Returns the highest edge ID in the graph, if any.
    #[must_use]
    pub fn max_edge_id(&self) -> Option<u64> {
        self.inner().max_edge_id()
    }

    /// Returns `true` when an edge with `edge_id` exists.
    #[must_use]
    pub fn edge_exists(&self, edge_id: u64) -> bool {
        self.inner().edge_exists(edge_id)
    }

    // -------------------------------------------------------------------------
    // Point retrieval (shared)
    // -------------------------------------------------------------------------

    /// Retrieves points by IDs, returning `None` for missing entries.
    #[must_use]
    pub fn get(&self, ids: &[u64]) -> Vec<Option<crate::point::Point>> {
        match self {
            Self::Vector(c) => c.get(ids),
            Self::Graph(c) => c.get(ids),
            Self::Metadata(c) => c.get(ids),
        }
    }

    /// Upserts points (vector + payload).
    ///
    /// For graph collections the payload is stored via the node-payload path
    /// (no vector update occurs since graph nodes have no embedding by default).
    ///
    /// # Errors
    ///
    /// Returns an error if storage fails.
    pub fn upsert(&self, points: Vec<crate::point::Point>) -> Result<()> {
        match self {
            Self::Vector(c) => c.upsert(points),
            Self::Graph(c) => {
                for p in points {
                    if let Some(payload) = p.payload.as_ref() {
                        c.upsert_node_payload(p.id, payload)?;
                    }
                }
                Ok(())
            }
            Self::Metadata(c) => c.upsert(points),
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
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use velesdb_core::{Database, GraphSchema};
    ///
    /// let db = Database::open("./data")?;
    /// db.create_graph_collection("edges", GraphSchema::schemaless())?;
    /// let any = db.get_any_collection("edges").expect("exists");
    /// assert!(any.is_graph());
    /// # Ok::<(), velesdb_core::Error>(())
    /// ```
    #[must_use]
    pub fn is_graph(&self) -> bool {
        matches!(self, Self::Graph(_))
    }

    /// Returns `true` if this collection is the [`Metadata`](Self::Metadata) variant.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use velesdb_core::Database;
    ///
    /// let db = Database::open("./data")?;
    /// db.create_metadata_collection("catalog")?;
    /// let any = db.get_any_collection("catalog").expect("exists");
    /// assert!(any.is_metadata());
    /// # Ok::<(), velesdb_core::Error>(())
    /// ```
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
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use velesdb_core::{Database, GraphSchema};
    ///
    /// let db = Database::open("./data")?;
    /// db.create_graph_collection("edges", GraphSchema::schemaless())?;
    /// let any = db.get_any_collection("edges").expect("exists");
    /// if let Some(g) = any.as_graph() {
    ///     let _ = g.edge_count();
    /// }
    /// # Ok::<(), velesdb_core::Error>(())
    /// ```
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
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use velesdb_core::Database;
    ///
    /// let db = Database::open("./data")?;
    /// db.create_metadata_collection("catalog")?;
    /// let any = db.get_any_collection("catalog").expect("exists");
    /// if let Some(m) = any.as_metadata() {
    ///     let _ = m.is_empty();
    /// }
    /// # Ok::<(), velesdb_core::Error>(())
    /// ```
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
    /// On the wrong variant, returns `Err(self)` so callers can recover
    /// ownership.
    ///
    /// # Errors
    ///
    /// Returns the original `AnyCollection` unchanged when the variant is
    /// [`Vector`](Self::Vector) or [`Metadata`](Self::Metadata).
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use velesdb_core::{Database, GraphSchema};
    ///
    /// let db = Database::open("./data")?;
    /// db.create_graph_collection("edges", GraphSchema::schemaless())?;
    /// let any = db.get_any_collection("edges").expect("exists");
    /// match any.into_graph() {
    ///     Ok(graph) => { let _ = graph.edge_count(); }
    ///     Err(_wrong_variant) => unreachable!("edges is a graph collection"),
    /// }
    /// # Ok::<(), velesdb_core::Error>(())
    /// ```
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
    /// On the wrong variant, returns `Err(self)` so callers can recover
    /// ownership.
    ///
    /// # Errors
    ///
    /// Returns the original `AnyCollection` unchanged when the variant is
    /// [`Vector`](Self::Vector) or [`Graph`](Self::Graph).
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use velesdb_core::Database;
    ///
    /// let db = Database::open("./data")?;
    /// db.create_metadata_collection("catalog")?;
    /// let any = db.get_any_collection("catalog").expect("exists");
    /// match any.into_metadata() {
    ///     Ok(meta) => assert!(meta.is_empty()),
    ///     Err(_wrong_variant) => unreachable!("catalog is a metadata collection"),
    /// }
    /// # Ok::<(), velesdb_core::Error>(())
    /// ```
    #[allow(clippy::result_large_err)]
    pub fn into_metadata(self) -> core::result::Result<MetadataCollection, Self> {
        match self {
            Self::Metadata(c) => Ok(c),
            other => Err(other),
        }
    }
}
