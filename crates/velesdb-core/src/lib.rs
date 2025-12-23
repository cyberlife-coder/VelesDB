//! # `VelesDB` Core
//!
//! High-performance vector database engine written in Rust.
//!
//! `VelesDB` is a local-first vector database designed for semantic search,
//! recommendation systems, and RAG (Retrieval-Augmented Generation) applications.
//!
//! ## Features
//!
//! - **Blazing Fast**: HNSW index with SIMD-optimized distance calculations
//! - **Persistent Storage**: Memory-mapped files for efficient disk access
//! - **Simple API**: Easy-to-use interface for vector operations
//!
//! ## Quick Start
//!
//! ```rust,ignore
//! use velesdb_core::{Database, Collection, DistanceMetric};
//!
//! // Create a new database
//! let db = Database::open("./data")?;
//!
//! // Create a collection
//! let collection = db.create_collection("documents", 768, DistanceMetric::Cosine)?;
//!
//! // Insert vectors
//! collection.upsert(vec![
//!     Point::new(1, vec![0.1, 0.2, ...], json!({"title": "Hello World"})),
//! ])?;
//!
//! // Search for similar vectors
//! let results = collection.search(&query_vector, 10)?;
//! ```

#![warn(missing_docs)]
#![warn(clippy::all)]
#![warn(clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]

pub mod collection;
pub mod column_store;
pub mod distance;
pub mod error;
pub mod filter;
pub mod half_precision;
pub mod index;
pub mod metrics;
pub mod perf_optimizations;
pub mod point;
pub mod quantization;
pub mod simd;
pub mod simd_avx512;
pub mod simd_explicit;
pub mod storage;
pub mod velesql;

pub use index::{HnswIndex, HnswParams, SearchQuality, VectorIndex};

pub use collection::Collection;
pub use distance::DistanceMetric;
pub use error::{Error, Result};
pub use filter::{Condition, Filter};
pub use point::Point;
pub use quantization::{BinaryQuantizedVector, QuantizedVector, StorageMode};

pub use column_store::{ColumnStore, ColumnType, ColumnValue, StringId, StringTable, TypedColumn};
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
        let mut collections = self.collections.write();

        if collections.contains_key(name) {
            return Err(Error::CollectionExists(name.to_string()));
        }

        let collection_path = self.data_dir.join(name);
        let collection = Collection::create(collection_path, dimension, metric)?;
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
