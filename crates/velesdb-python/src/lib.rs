// Python SDK - pedantic/nursery lints relaxed for PyO3 FFI boundary
#![allow(clippy::pedantic)]
#![allow(clippy::nursery)]
#![allow(clippy::useless_conversion)]
//! Python bindings for `VelesDB` vector database.
//!
//! This module provides a Pythonic interface to VelesDB using PyO3.
//!
//! # Example
//!
//! ```python
//! import velesdb
//!
//! # Open database
//! db = velesdb.Database("./my_data")
//!
//! # Create collection
//! collection = db.create_collection("documents", dimension=768, metric="cosine")
//!
//! # Insert vectors
//! collection.upsert([
//!     {"id": 1, "vector": [0.1, 0.2, ...], "payload": {"title": "Doc 1"}}
//! ])
//!
//! # Search
//! results = collection.search([0.1, 0.2, ...], top_k=10)
//! ```

mod agent;
mod collection;
mod collection_helpers;
mod graph;
mod graph_store;
mod utils;
mod velesql;

pub use collection::Collection;
pub use graph::{dict_to_edge, dict_to_node, edge_to_dict, node_to_dict, traversal_to_dict};
pub use graph_store::{GraphStore, StreamingConfig, TraversalResult};

use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use std::path::PathBuf;
use std::sync::Arc;

use utils::{parse_metric, parse_storage_mode};
use velesdb_core::{Database as CoreDatabase, FusionStrategy as CoreFusionStrategy};

/// Fusion strategy for combining results from multiple vector searches.
///
/// Example:
///     >>> # Average fusion
///     >>> strategy = FusionStrategy.average()
///     >>> # RRF with default k=60
///     >>> strategy = FusionStrategy.rrf()
///     >>> # Weighted fusion
///     >>> strategy = FusionStrategy.weighted(avg_weight=0.6, max_weight=0.3, hit_weight=0.1)
#[pyclass]
#[derive(Clone)]
pub struct FusionStrategy {
    inner: CoreFusionStrategy,
}

#[pymethods]
impl FusionStrategy {
    /// Create an Average fusion strategy.
    ///
    /// Computes the mean score for each document across all queries.
    ///
    /// Returns:
    ///     FusionStrategy: Average fusion strategy
    ///
    /// Example:
    ///     >>> strategy = FusionStrategy.average()
    #[staticmethod]
    fn average() -> Self {
        Self {
            inner: CoreFusionStrategy::Average,
        }
    }

    /// Create a Maximum fusion strategy.
    ///
    /// Takes the maximum score for each document across all queries.
    ///
    /// Returns:
    ///     FusionStrategy: Maximum fusion strategy
    ///
    /// Example:
    ///     >>> strategy = FusionStrategy.maximum()
    #[staticmethod]
    fn maximum() -> Self {
        Self {
            inner: CoreFusionStrategy::Maximum,
        }
    }

    /// Create a Reciprocal Rank Fusion (RRF) strategy.
    ///
    /// Uses position-based scoring: score = Σ 1/(k + rank)
    /// This is robust to score scale differences between queries.
    ///
    /// Args:
    ///     k: Ranking constant (default: 60). Lower k gives more weight to top ranks.
    ///
    /// Returns:
    ///     FusionStrategy: RRF fusion strategy
    ///
    /// Example:
    ///     >>> strategy = FusionStrategy.rrf()  # k=60
    ///     >>> strategy = FusionStrategy.rrf(k=30)  # More emphasis on top ranks
    #[staticmethod]
    #[pyo3(signature = (k = 60))]
    fn rrf(k: u32) -> Self {
        Self {
            inner: CoreFusionStrategy::RRF { k },
        }
    }

    /// Create a Weighted fusion strategy.
    ///
    /// Combines average score, maximum score, and hit ratio with custom weights.
    /// Formula: score = avg_weight * avg + max_weight * max + hit_weight * hit_ratio
    ///
    /// Args:
    ///     avg_weight: Weight for average score (0.0-1.0)
    ///     max_weight: Weight for maximum score (0.0-1.0)
    ///     hit_weight: Weight for hit ratio (0.0-1.0)
    ///
    /// Returns:
    ///     FusionStrategy: Weighted fusion strategy
    ///
    /// Raises:
    ///     ValueError: If weights don't sum to 1.0 or are negative
    ///
    /// Example:
    ///     >>> strategy = FusionStrategy.weighted(
    ///     ...     avg_weight=0.6,
    ///     ...     max_weight=0.3,
    ///     ...     hit_weight=0.1
    ///     ... )
    #[staticmethod]
    #[pyo3(signature = (avg_weight, max_weight, hit_weight))]
    fn weighted(avg_weight: f32, max_weight: f32, hit_weight: f32) -> PyResult<Self> {
        CoreFusionStrategy::weighted(avg_weight, max_weight, hit_weight)
            .map(|inner| Self { inner })
            .map_err(|e| PyValueError::new_err(format!("{e}")))
    }

    /// Create a Relative Score Fusion (RSF) strategy.
    ///
    /// Linearly combines dense and sparse scores with the given weights.
    /// Useful for hybrid dense+sparse search.
    ///
    /// Args:
    ///     dense_weight: Weight for dense vector scores (0.0-1.0)
    ///     sparse_weight: Weight for sparse scores (0.0-1.0)
    ///
    /// Returns:
    ///     FusionStrategy: Relative score fusion strategy
    ///
    /// Raises:
    ///     ValueError: If weights are invalid
    ///
    /// Example:
    ///     >>> strategy = FusionStrategy.relative_score(0.7, 0.3)
    #[staticmethod]
    #[pyo3(signature = (dense_weight, sparse_weight))]
    fn relative_score(dense_weight: f32, sparse_weight: f32) -> PyResult<Self> {
        CoreFusionStrategy::relative_score(dense_weight, sparse_weight)
            .map(|inner| Self { inner })
            .map_err(|e| PyValueError::new_err(format!("{e}")))
    }

    fn __repr__(&self) -> String {
        match &self.inner {
            CoreFusionStrategy::Average => "FusionStrategy.average()".to_string(),
            CoreFusionStrategy::Maximum => "FusionStrategy.maximum()".to_string(),
            CoreFusionStrategy::RRF { k } => format!("FusionStrategy.rrf(k={k})"),
            CoreFusionStrategy::Weighted {
                avg_weight,
                max_weight,
                hit_weight,
            } => format!(
                "FusionStrategy.weighted(avg_weight={avg_weight}, max_weight={max_weight}, hit_weight={hit_weight})"
            ),
            CoreFusionStrategy::RelativeScore {
                dense_weight,
                sparse_weight,
            } => format!(
                "FusionStrategy.relative_score(dense_weight={dense_weight}, sparse_weight={sparse_weight})"
            ),
        }
    }
}

impl FusionStrategy {
    /// Get the inner CoreFusionStrategy.
    pub fn inner(&self) -> CoreFusionStrategy {
        self.inner.clone()
    }
}

/// VelesDB Database - the main entry point for interacting with VelesDB.
///
/// Example:
///     >>> db = velesdb.Database("./my_data")
///     >>> collections = db.list_collections()
#[pyclass]
pub struct Database {
    inner: CoreDatabase,
    path: PathBuf,
}

#[pymethods]
impl Database {
    /// Create or open a VelesDB database at the specified path.
    ///
    /// Args:
    ///     path: Directory path for database storage
    ///
    /// Returns:
    ///     Database instance
    ///
    /// Example:
    ///     >>> db = velesdb.Database("./my_vectors")
    #[new]
    #[pyo3(signature = (path))]
    fn new(path: &str) -> PyResult<Self> {
        let path_buf = PathBuf::from(path);
        let db = CoreDatabase::open(&path_buf)
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to open database: {}", e)))?;
        Ok(Self {
            inner: db,
            path: path_buf,
        })
    }

    /// Create a new vector collection.
    ///
    /// Args:
    ///     name: Collection name
    ///     dimension: Vector dimension (e.g., 768 for BERT embeddings)
    ///     metric: Distance metric - "cosine", "euclidean", "dot", "hamming", or "jaccard"
    ///             (default: "cosine")
    ///     storage_mode: Storage mode - "full", "sq8", or "binary" (default: "full")
    ///                   - "full": Full f32 precision
    ///                   - "sq8": 8-bit scalar quantization (4x memory reduction)
    ///                   - "binary": 1-bit binary quantization (32x memory reduction)
    ///
    /// Returns:
    ///     Collection instance
    ///
    /// Example:
    ///     >>> collection = db.create_collection("documents", dimension=768, metric="cosine")
    ///     >>> # With SQ8 quantization for memory savings:
    ///     >>> quantized = db.create_collection("embeddings", dimension=768, storage_mode="sq8")
    #[pyo3(signature = (name, dimension, metric = "cosine", storage_mode = "full"))]
    fn create_collection(
        &self,
        name: &str,
        dimension: usize,
        metric: &str,
        storage_mode: &str,
    ) -> PyResult<Collection> {
        let distance_metric = parse_metric(metric)?;
        let mode = parse_storage_mode(storage_mode)?;

        self.inner
            .create_vector_collection_with_options(name, dimension, distance_metric, mode)
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to create collection: {}", e)))?;

        let collection = self
            .inner
            .get_collection(name)
            .ok_or_else(|| PyRuntimeError::new_err("Collection not found after creation"))?;

        Ok(Collection::new(Arc::new(collection), name.to_string()))
    }

    /// Get an existing collection by name.
    ///
    /// Args:
    ///     name: Collection name
    ///
    /// Returns:
    ///     Collection instance or None if not found
    ///
    /// Example:
    ///     >>> collection = db.get_collection("documents")
    #[pyo3(signature = (name))]
    fn get_collection(&self, name: &str) -> PyResult<Option<Collection>> {
        match self.inner.get_collection(name) {
            Some(collection) => Ok(Some(Collection::new(
                Arc::new(collection),
                name.to_string(),
            ))),
            None => Ok(None),
        }
    }

    /// List all collection names in the database.
    ///
    /// Returns:
    ///     List of collection names
    ///
    /// Example:
    ///     >>> names = db.list_collections()
    ///     >>> print(names)  # ['documents', 'images']
    fn list_collections(&self) -> Vec<String> {
        self.inner.list_collections()
    }

    /// Delete a collection by name.
    ///
    /// Args:
    ///     name: Collection name to delete
    ///
    /// Example:
    ///     >>> db.delete_collection("old_collection")
    #[pyo3(signature = (name))]
    fn delete_collection(&self, name: &str) -> PyResult<()> {
        self.inner
            .delete_collection(name)
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to delete collection: {}", e)))
    }

    /// Create a metadata-only collection (no vectors, no HNSW index).
    ///
    /// Metadata-only collections are optimized for storing reference data,
    /// catalogs, and other non-vector data. They support CRUD operations
    /// and VelesQL queries on payload, but NOT vector search.
    ///
    /// Args:
    ///     name: Collection name
    ///
    /// Returns:
    ///     Collection instance
    ///
    /// Example:
    ///     >>> products = db.create_metadata_collection("products")
    ///     >>> products.upsert_metadata([
    ///     ...     {"id": 1, "payload": {"name": "Widget", "price": 9.99}}
    ///     ... ])
    #[pyo3(signature = (name))]
    fn create_metadata_collection(&self, name: &str) -> PyResult<Collection> {
        self.inner.create_metadata_collection(name).map_err(|e| {
            PyRuntimeError::new_err(format!("Failed to create metadata collection: {e}"))
        })?;

        // Metadata collections are registered in the legacy `collections`
        // registry, so resolve from there after creation.
        let collection = self
            .inner
            .get_collection(name)
            .ok_or_else(|| PyRuntimeError::new_err("Collection not found after creation"))?;

        Ok(Collection::new(Arc::new(collection), name.to_string()))
    }

    /// Create an AgentMemory instance for AI agent workflows.
    ///
    /// Args:
    ///     dimension: Embedding dimension (default: 384)
    ///
    /// Returns:
    ///     AgentMemory instance with semantic, episodic, and procedural subsystems
    ///
    /// Example:
    ///     >>> memory = db.agent_memory()
    ///     >>> memory.semantic.store(1, "Paris is in France", embedding)
    #[pyo3(signature = (dimension = None))]
    fn agent_memory(&self, dimension: Option<usize>) -> PyResult<agent::AgentMemory> {
        agent::AgentMemory::new(self, dimension)
    }

    /// Train product quantization on a collection.
    ///
    /// Builds PQ codebooks from existing vectors, enabling compressed
    /// storage and faster ADC-based search.
    ///
    /// Args:
    ///     collection_name: Name of the collection to train on.
    ///     m: Number of subspaces (dimension must be divisible by m). Default: 8.
    ///     k: Number of centroids per subspace. Default: 256.
    ///     opq: Whether to use Optimized Product Quantization. Default: False.
    ///
    /// Returns:
    ///     Status message from the training operation.
    ///
    /// Raises:
    ///     RuntimeError: If training fails (e.g., insufficient data, bad params).
    ///
    /// Example:
    ///     >>> db.train_pq("documents", m=8, k=256)
    ///     >>> db.train_pq("documents", m=16, k=128, opq=True)
    #[pyo3(signature = (collection_name, m=8, k=256, opq=false))]
    fn train_pq(&self, collection_name: &str, m: usize, k: usize, opq: bool) -> PyResult<String> {
        // Validate collection_name to prevent VelesQL injection via string interpolation.
        // Only alphanumeric characters and underscores are accepted (same constraint
        // as the collection name rules enforced at creation time).
        if !collection_name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            return Err(PyValueError::new_err(format!(
                "Invalid collection name '{collection_name}': only ASCII letters, digits, \
                 and underscores are allowed"
            )));
        }

        let mut query = format!("TRAIN QUANTIZER ON {collection_name} WITH (m={m}, k={k}");
        if opq {
            query.push_str(", type=opq");
        }
        query.push(')');

        let parsed = velesdb_core::velesql::Parser::parse(&query).map_err(|e| {
            PyValueError::new_err(format!("Failed to construct TRAIN query: {}", e.message))
        })?;

        let empty_params = std::collections::HashMap::new();
        let results = self
            .inner
            .execute_query(&parsed, &empty_params)
            .map_err(|e| PyRuntimeError::new_err(format!("PQ training failed: {e}")))?;

        Ok(format!("PQ training complete: {} results", results.len()))
    }
}

impl Database {
    /// Get a reference to the inner CoreDatabase.
    pub fn inner(&self) -> &CoreDatabase {
        &self.inner
    }

    /// Get the database path.
    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    /// Open an independent `Arc<CoreDatabase>` handle from the same path.
    ///
    /// Used by subsystems (e.g., AgentMemory) that need `Arc` ownership.
    /// Each call creates a **separate** in-memory instance that reads/writes
    /// the same on-disk data. Changes are visible across handles after flush.
    pub fn open_shared(&self) -> std::result::Result<Arc<CoreDatabase>, String> {
        CoreDatabase::open(&self.path)
            .map(Arc::new)
            .map_err(|e| format!("Failed to open shared database: {e}"))
    }
}

/// Search result from a vector query.
#[pyclass]
pub struct SearchResult {
    #[pyo3(get)]
    id: u64,
    #[pyo3(get)]
    score: f32,
    #[pyo3(get)]
    payload: PyObject,
}

/// VelesDB - A high-performance vector database for AI applications.
///
/// Example:
///     >>> import velesdb
///     >>> db = velesdb.Database("./my_data")
///     >>> collection = db.create_collection("docs", dimension=768)
///     >>> collection.upsert([{"id": 1, "vector": [...], "payload": {"title": "Doc"}}])
///     >>> results = collection.search([...], top_k=10)
#[pymodule]
fn velesdb(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Database>()?;
    m.add_class::<Collection>()?;
    m.add_class::<SearchResult>()?;
    m.add_class::<FusionStrategy>()?;

    // Graph classes (EPIC-016/US-030, US-032)
    m.add_class::<GraphStore>()?;
    m.add_class::<StreamingConfig>()?;
    m.add_class::<TraversalResult>()?;

    // Agent memory classes (EPIC-010/US-005)
    m.add_class::<agent::AgentMemory>()?;
    m.add_class::<agent::PySemanticMemory>()?;
    m.add_class::<agent::PyEpisodicMemory>()?;
    m.add_class::<agent::PyProceduralMemory>()?;

    // VelesQL query language classes (EPIC-056/US-001)
    velesql::register_velesql_module(m)?;

    // Add version info
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;

    Ok(())
}
