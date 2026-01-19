#![allow(clippy::useless_conversion)]
#![allow(clippy::pedantic)] // PyO3 binding code has many style differences
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

mod collection;
mod utils;

pub use collection::Collection;

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
        let db = CoreDatabase::open(PathBuf::from(path))
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to open database: {}", e)))?;
        Ok(Self { inner: db })
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
            .create_collection_with_options(name, dimension, distance_metric, mode)
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
        use velesdb_core::CollectionType;

        self.inner
            .create_collection_typed(name, &CollectionType::MetadataOnly)
            .map_err(|e| {
                PyRuntimeError::new_err(format!("Failed to create metadata collection: {e}"))
            })?;

        let collection = self
            .inner
            .get_collection(name)
            .ok_or_else(|| PyRuntimeError::new_err("Collection not found after creation"))?;

        Ok(Collection::new(Arc::new(collection), name.to_string()))
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

    // Add version info
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;

    Ok(())
}
