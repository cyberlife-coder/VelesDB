//! # `VelesDB` Core
//!
//! High-performance vector database engine written in Rust.
//!
//! `VelesDB` is a local-first vector database designed for semantic search,
//! recommendation systems, and RAG (Retrieval-Augmented Generation) applications.
//!
//! ## Features
//!
//! - **Blazing Fast**: HNSW index with explicit SIMD (4x faster)
//! - **5 Distance Metrics**: Cosine, Euclidean, Dot Product, Hamming, Jaccard
//! - **Hybrid Search**: Vector + BM25 full-text with RRF fusion
//! - **Quantization**: SQ8 (4x) and Binary (32x) memory compression
//! - **Persistent Storage**: Memory-mapped files for efficient disk access
//!
//! ## Quick Start
//!
//! ```rust,ignore
//! use velesdb_core::{Database, DistanceMetric, Point, StorageMode};
//! use serde_json::json;
//!
//! // Create a new database
//! let db = Database::open("./data")?;
//!
//! // Create a collection (all 5 metrics available)
//! db.create_collection("documents", 768, DistanceMetric::Cosine)?;
//! // Or with quantization: DistanceMetric::Hamming + StorageMode::Binary
//!
//! let collection = db.get_collection("documents").unwrap();
//!
//! // Insert vectors (upsert takes ownership)
//! collection.upsert(vec![
//!     Point::new(1, vec![0.1; 768], Some(json!({"title": "Hello World"}))),
//! ])?;
//!
//! // Search for similar vectors
//! let results = collection.search(&query_vector, 10)?;
//!
//! // Hybrid search (vector + text)
//! let hybrid = collection.hybrid_search(&query_vector, "hello", 5, Some(0.7))?;
//! ```

#![warn(missing_docs)]
#![warn(clippy::all)]
#![warn(clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]

pub mod alloc_guard;
#[cfg(test)]
mod alloc_guard_tests;
pub mod collection;
pub mod column_store;
#[cfg(test)]
mod column_store_tests;
pub mod config;
#[cfg(test)]
mod config_tests;
pub mod distance;
#[cfg(test)]
mod distance_tests;
pub mod error;
#[cfg(test)]
mod error_tests;
pub mod filter;
#[cfg(test)]
mod filter_tests;
pub mod fusion;
pub mod gpu;
#[cfg(test)]
mod gpu_tests;
pub mod half_precision;
#[cfg(test)]
mod half_precision_tests;
pub mod index;
pub mod metrics;
#[cfg(test)]
mod metrics_tests;
pub mod perf_optimizations;
pub mod point;
#[cfg(test)]
mod point_tests;
pub mod quantization;
#[cfg(test)]
mod quantization_tests;
pub mod simd;
pub mod simd_avx512;
#[cfg(test)]
mod simd_avx512_tests;
pub mod simd_dispatch;
pub mod simd_explicit;
#[cfg(test)]
mod simd_explicit_tests;
pub mod simd_native;
pub mod storage;
pub mod vector_ref;
pub mod velesql;

pub use index::{HnswIndex, HnswParams, SearchQuality, VectorIndex};

pub use collection::Collection;
pub use distance::DistanceMetric;
pub use error::{Error, Result};
pub use filter::{Condition, Filter};
pub use point::{Point, SearchResult};
pub use quantization::{
    cosine_similarity_quantized, cosine_similarity_quantized_simd, dot_product_quantized,
    dot_product_quantized_simd, euclidean_squared_quantized, euclidean_squared_quantized_simd,
    BinaryQuantizedVector, QuantizedVector, StorageMode,
};

pub use column_store::{ColumnStore, ColumnType, ColumnValue, StringId, StringTable, TypedColumn};
pub use config::{
    ConfigError, HnswConfig, LimitsConfig, LoggingConfig, QuantizationConfig, SearchConfig,
    SearchMode, ServerConfig, StorageConfig, VelesConfig,
};
pub use fusion::{FusionError, FusionStrategy};
pub use metrics::{
    average_metrics, compute_latency_percentiles, hit_rate, mean_average_precision, mrr, ndcg_at_k,
    precision_at_k, recall_at_k, LatencyStats,
};

/// Database instance managing collections and storage.
pub struct Database {
    /// Path to the data directory
    data_dir: std::path::PathBuf,
    /// Collections managed by this database
    collections: parking_lot::RwLock<std::collections::HashMap<String, Collection>>,
}

impl Database {
    /// Opens or creates a database at the specified path.
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the data directory
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created or accessed.
    pub fn open<P: AsRef<std::path::Path>>(path: P) -> Result<Self> {
        let data_dir = path.as_ref().to_path_buf();
        std::fs::create_dir_all(&data_dir)?;

        Ok(Self {
            data_dir,
            collections: parking_lot::RwLock::new(std::collections::HashMap::new()),
        })
    }

    /// Creates a new collection with the specified parameters.
    ///
    /// # Arguments
    ///
    /// * `name` - Unique name for the collection
    /// * `dimension` - Vector dimension (e.g., 768 for many embedding models)
    /// * `metric` - Distance metric to use for similarity calculations
    ///
    /// # Errors
    ///
    /// Returns an error if a collection with the same name already exists.
    pub fn create_collection(
        &self,
        name: &str,
        dimension: usize,
        metric: DistanceMetric,
    ) -> Result<()> {
        self.create_collection_with_options(name, dimension, metric, StorageMode::default())
    }

    /// Creates a new collection with custom storage options.
    ///
    /// # Arguments
    ///
    /// * `name` - Unique name for the collection
    /// * `dimension` - Vector dimension
    /// * `metric` - Distance metric
    /// * `storage_mode` - Vector storage mode (Full, SQ8, Binary)
    ///
    /// # Errors
    ///
    /// Returns an error if a collection with the same name already exists.
    pub fn create_collection_with_options(
        &self,
        name: &str,
        dimension: usize,
        metric: DistanceMetric,
        storage_mode: StorageMode,
    ) -> Result<()> {
        let mut collections = self.collections.write();

        if collections.contains_key(name) {
            return Err(Error::CollectionExists(name.to_string()));
        }

        let collection_path = self.data_dir.join(name);
        let collection =
            Collection::create_with_options(collection_path, dimension, metric, storage_mode)?;
        collections.insert(name.to_string(), collection);

        Ok(())
    }

    /// Gets a reference to a collection by name.
    ///
    /// # Arguments
    ///
    /// * `name` - Name of the collection
    ///
    /// # Returns
    ///
    /// Returns `None` if the collection does not exist.
    pub fn get_collection(&self, name: &str) -> Option<Collection> {
        self.collections.read().get(name).cloned()
    }

    /// Lists all collection names in the database.
    pub fn list_collections(&self) -> Vec<String> {
        self.collections.read().keys().cloned().collect()
    }

    /// Deletes a collection by name.
    ///
    /// # Arguments
    ///
    /// * `name` - Name of the collection to delete
    ///
    /// # Errors
    ///
    /// Returns an error if the collection does not exist.
    pub fn delete_collection(&self, name: &str) -> Result<()> {
        let mut collections = self.collections.write();

        if collections.remove(name).is_none() {
            return Err(Error::CollectionNotFound(name.to_string()));
        }

        let collection_path = self.data_dir.join(name);
        if collection_path.exists() {
            std::fs::remove_dir_all(collection_path)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_database_open() {
        let dir = tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();
        assert!(db.list_collections().is_empty());
    }

    #[test]
    fn test_create_collection() {
        let dir = tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();

        db.create_collection("test", 768, DistanceMetric::Cosine)
            .unwrap();

        assert_eq!(db.list_collections(), vec!["test"]);
    }

    #[test]
    fn test_duplicate_collection_error() {
        let dir = tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();

        db.create_collection("test", 768, DistanceMetric::Cosine)
            .unwrap();

        let result = db.create_collection("test", 768, DistanceMetric::Cosine);
        assert!(result.is_err());
    }

    #[test]
    fn test_get_collection() {
        let dir = tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();

        // Non-existent collection returns None
        assert!(db.get_collection("nonexistent").is_none());

        // Create and retrieve collection
        db.create_collection("test", 768, DistanceMetric::Cosine)
            .unwrap();

        let collection = db.get_collection("test");
        assert!(collection.is_some());

        let config = collection.unwrap().config();
        assert_eq!(config.dimension, 768);
        assert_eq!(config.metric, DistanceMetric::Cosine);
    }

    #[test]
    fn test_delete_collection() {
        let dir = tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();

        db.create_collection("to_delete", 768, DistanceMetric::Cosine)
            .unwrap();
        assert_eq!(db.list_collections().len(), 1);

        // Delete the collection
        db.delete_collection("to_delete").unwrap();
        assert!(db.list_collections().is_empty());
        assert!(db.get_collection("to_delete").is_none());
    }

    #[test]
    fn test_delete_nonexistent_collection() {
        let dir = tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();

        let result = db.delete_collection("nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn test_multiple_collections() {
        let dir = tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();

        db.create_collection("coll1", 128, DistanceMetric::Cosine)
            .unwrap();
        db.create_collection("coll2", 256, DistanceMetric::Euclidean)
            .unwrap();
        db.create_collection("coll3", 768, DistanceMetric::DotProduct)
            .unwrap();

        let collections = db.list_collections();
        assert_eq!(collections.len(), 3);
        assert!(collections.contains(&"coll1".to_string()));
        assert!(collections.contains(&"coll2".to_string()));
        assert!(collections.contains(&"coll3".to_string()));
    }
}
