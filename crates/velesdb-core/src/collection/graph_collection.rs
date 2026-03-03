//! `GraphCollection`: knowledge graph with optional node embeddings.
//!
//! This type owns a [`GraphEngine`] (edges + property/range indexes) and an optional
//! [`VectorEngine`] for collections where nodes also carry embeddings.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::RwLock;

use crate::collection::graph::{GraphEdge, GraphSchema, TraversalConfig, TraversalResult};
use crate::collection::types::{Collection, CollectionConfig};
use crate::distance::DistanceMetric;
use crate::engine::graph::GraphEngine;
use crate::engine::payload::PayloadEngine;
use crate::engine::vector::VectorEngine;
use crate::error::{Error, Result};
use crate::guardrails::GuardRails;
use crate::point::{Point, SearchResult};
use crate::quantization::StorageMode;
use crate::velesql::QueryPlanner;

/// A graph collection storing typed relationships between nodes.
///
/// Node embeddings are optional: if `dimension` is `None`, no vector index is created.
///
/// # Examples
///
/// ```rust,no_run
/// use velesdb_core::{GraphCollection, GraphSchema, GraphEdge};
///
/// let coll = GraphCollection::create(
///     "./data/kg".into(),
///     "knowledge",
///     None,            // no embeddings
///     Default::default(), // Cosine (unused)
///     GraphSchema::schemaless(),
/// )?;
///
/// let edge = GraphEdge::new(1, 100, 200, "KNOWS")?;
/// coll.add_edge(edge)?;
/// # Ok::<(), velesdb_core::Error>(())
/// ```
#[derive(Clone)]
pub struct GraphCollection {
    /// Path to the collection directory on disk.
    pub(crate) path: PathBuf,
    /// Collection metadata.
    pub(crate) config: Arc<RwLock<CollectionConfig>>,
    /// Graph engine: edges + property/range indexes + traversal.
    pub(crate) graph: GraphEngine,
    /// Optional vector engine (when nodes have embeddings).
    pub(crate) embeddings: Option<VectorEngine>,
    /// Payload engine: node property storage + BM25.
    pub(crate) payload: PayloadEngine,
    /// Guard-rails (reserved for future native executor).
    #[allow(dead_code)]
    pub(crate) guard_rails: Arc<GuardRails>,
    /// Query planner (reserved for future native executor).
    #[allow(dead_code)]
    pub(crate) query_planner: Arc<QueryPlanner>,
    /// Graph schema (schemaless or strict).
    pub(crate) schema: GraphSchema,
    /// Shared executor for VelesQL queries.
    pub(crate) inner: Collection,
}

impl GraphCollection {
    // -------------------------------------------------------------------------
    // Lifecycle
    // -------------------------------------------------------------------------

    /// Creates a new `GraphCollection`.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created or storage fails.
    pub fn create(
        path: PathBuf,
        name: &str,
        dimension: Option<usize>,
        metric: DistanceMetric,
        schema: GraphSchema,
    ) -> Result<Self> {
        std::fs::create_dir_all(&path)?;

        let config = CollectionConfig {
            name: name.to_string(),
            dimension: dimension.unwrap_or(0),
            metric,
            point_count: 0,
            storage_mode: StorageMode::Full,
            metadata_only: false,
        };

        let graph = GraphEngine::new();
        let payload = PayloadEngine::create(&path)?;
        let embeddings = if let Some(dim) = dimension {
            Some(VectorEngine::create(&path, dim, metric, StorageMode::Full)?)
        } else {
            None
        };

        // Build inner AFTER engines create the directory and files.
        // Use create_metadata_only so config.json is written once.
        let inner = Collection::create_metadata_only(path.clone(), name)?;

        let coll = Self {
            path,
            config: Arc::new(RwLock::new(config)),
            graph,
            embeddings,
            payload,
            guard_rails: Arc::new(GuardRails::default()),
            query_planner: Arc::new(QueryPlanner::new()),
            schema,
            inner,
        };
        Ok(coll)
    }

    /// Opens an existing `GraphCollection` from disk.
    ///
    /// # Errors
    ///
    /// Returns an error if config or storage cannot be opened.
    pub fn open(path: PathBuf) -> Result<Self> {
        let config_path = path.join("config.json");
        let config_data = std::fs::read_to_string(&config_path)?;
        let config: CollectionConfig =
            serde_json::from_str(&config_data).map_err(|e| Error::Serialization(e.to_string()))?;

        let graph = GraphEngine::open(&path);
        let payload = PayloadEngine::open(&path)?;
        let embeddings = if config.dimension > 0 {
            Some(VectorEngine::open(
                &path,
                config.dimension,
                config.metric,
                config.storage_mode,
            )?)
        } else {
            None
        };

        // Schema: currently read from config; future work will persist schema separately.
        let schema = GraphSchema::schemaless();

        let inner = Collection::open(path.clone())?;

        Ok(Self {
            path,
            config: Arc::new(RwLock::new(config)),
            graph,
            embeddings,
            payload,
            guard_rails: Arc::new(GuardRails::default()),
            query_planner: Arc::new(QueryPlanner::new()),
            schema,
            inner,
        })
    }

    pub(crate) fn save_config(&self) -> Result<()> {
        self.inner.save_config()
    }

    /// Flushes all engines and saves the config.
    ///
    /// # Errors
    ///
    /// Returns an error if any flush operation fails.
    pub fn flush(&self) -> Result<()> {
        self.save_config()?;
        self.graph.flush(&self.path)?;
        self.payload.flush()?;
        if let Some(ref ve) = self.embeddings {
            ve.flush(&self.path)?;
        }
        Ok(())
    }

    // -------------------------------------------------------------------------
    // Metadata
    // -------------------------------------------------------------------------

    /// Returns the collection name.
    #[must_use]
    pub fn name(&self) -> String {
        self.config.read().name.clone()
    }

    /// Returns the graph schema.
    #[must_use]
    pub fn schema(&self) -> &GraphSchema {
        &self.schema
    }

    /// Returns `true` if this collection stores node embeddings.
    #[must_use]
    pub fn has_embeddings(&self) -> bool {
        self.embeddings.is_some()
    }

    // -------------------------------------------------------------------------
    // Graph operations
    // -------------------------------------------------------------------------

    /// Adds an edge between two nodes.
    ///
    /// # Errors
    ///
    /// Returns `Error::EdgeExists` if an edge with the same ID already exists.
    pub fn add_edge(&self, edge: GraphEdge) -> Result<()> {
        self.graph.add_edge(edge)
    }

    /// Returns edges, optionally filtered by label.
    #[must_use]
    pub fn get_edges(&self, label: Option<&str>) -> Vec<GraphEdge> {
        self.graph.get_edges(label)
    }

    /// Returns all outgoing edges from a node.
    #[must_use]
    pub fn get_outgoing(&self, node_id: u64) -> Vec<GraphEdge> {
        self.graph.get_outgoing(node_id)
    }

    /// Returns all incoming edges to a node.
    #[must_use]
    pub fn get_incoming(&self, node_id: u64) -> Vec<GraphEdge> {
        self.graph.get_incoming(node_id)
    }

    /// Returns `(in_degree, out_degree)` for a node.
    #[must_use]
    pub fn node_degree(&self, node_id: u64) -> (usize, usize) {
        self.graph.node_degree(node_id)
    }

    /// Performs BFS traversal from a source node.
    #[must_use]
    pub fn traverse_bfs(&self, source_id: u64, config: &TraversalConfig) -> Vec<TraversalResult> {
        self.graph.traverse_bfs(source_id, config)
    }

    /// Performs DFS traversal from a source node.
    #[must_use]
    pub fn traverse_dfs(&self, source_id: u64, config: &TraversalConfig) -> Vec<TraversalResult> {
        self.graph.traverse_dfs(source_id, config)
    }

    // -------------------------------------------------------------------------
    // Payload / node properties
    // -------------------------------------------------------------------------

    /// Stores node payload (properties).
    ///
    /// # Errors
    ///
    /// Returns an error if storage fails.
    pub fn store_node_payload(&self, node_id: u64, payload: &serde_json::Value) -> Result<()> {
        self.payload.store(node_id, Some(payload), None)
    }

    /// Retrieves node payload.
    ///
    /// # Errors
    ///
    /// Returns an error if retrieval fails.
    pub fn get_node_payload(&self, node_id: u64) -> Result<Option<serde_json::Value>> {
        self.payload.retrieve(node_id)
    }

    // -------------------------------------------------------------------------
    // Optional embedding search
    // -------------------------------------------------------------------------

    /// Searches for similar nodes by embedding (only available if `has_embeddings()`).
    ///
    /// # Errors
    ///
    /// Returns `Error::VectorNotAllowed` if this collection has no embeddings,
    /// or `Error::DimensionMismatch` if the query dimension is wrong.
    pub fn search_by_embedding(&self, query: &[f32], k: usize) -> Result<Vec<SearchResult>> {
        let ve = self
            .embeddings
            .as_ref()
            .ok_or_else(|| Error::VectorNotAllowed(self.config.read().name.clone()))?;
        let dim = self.config.read().dimension;
        if query.len() != dim {
            return Err(Error::DimensionMismatch {
                expected: dim,
                actual: query.len(),
            });
        }
        let ids = ve.search(query, k);
        let results = ids
            .into_iter()
            .filter_map(|(id, score)| {
                let vector = ve.retrieve_vector(id)?;
                let payload = self.payload.retrieve(id).ok().flatten();
                Some(SearchResult::new(
                    Point {
                        id,
                        vector,
                        payload,
                    },
                    score,
                ))
            })
            .collect();
        Ok(results)
    }

    // -------------------------------------------------------------------------
    // VelesQL
    // -------------------------------------------------------------------------

    /// Executes a `VelesQL` query (delegates to legacy `Collection` during migration).
    ///
    /// # Errors
    ///
    /// Returns an error if the query is invalid or execution fails.
    pub fn execute_query(
        &self,
        query: &crate::velesql::Query,
        params: &HashMap<String, serde_json::Value>,
    ) -> Result<Vec<SearchResult>> {
        self.inner.execute_query(query, params)
    }
}
