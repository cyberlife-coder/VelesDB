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
//! ```rust,no_run
//! use velesdb_core::{Database, DistanceMetric, Point, StorageMode};
//! use serde_json::json;
//!
//! fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Create a new database
//!     let db = Database::open("./data")?;
//!
//!     // Create a collection (all 5 metrics available)
//!     db.create_collection("documents", 768, DistanceMetric::Cosine)?;
//!     // Or with quantization: DistanceMetric::Hamming + StorageMode::Binary
//!
//!     let collection = db.get_collection("documents").ok_or("Collection not found")?;
//!
//!     // Insert vectors (upsert takes ownership)
//!     collection.upsert(vec![
//!         Point::new(1, vec![0.1; 768], Some(json!({"title": "Hello World"}))),
//!     ])?;
//!
//!     // Search for similar vectors
//!     let query_vector = vec![0.1; 768];
//!     let results = collection.search(&query_vector, 10)?;
//!
//!     // Hybrid search (vector + text)
//!     let hybrid = collection.hybrid_search(&query_vector, "hello", 5, Some(0.7))?;
//!     # Ok(())
//! }
//! ```

#![warn(missing_docs)]
#![warn(clippy::all)]
#![warn(clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]
// =============================================================================
// NUMERIC CAST LINTS - USE WITH CAUTION
// =============================================================================
// These are allowed globally for SIMD/performance code but can hide real bugs.
// RECOMMENDATION: Prefer local #[allow(...)] on specific functions instead.
// Review PR #163 FLAG-1: These may mask truncation/overflow bugs elsewhere.
//
// For new code: Use try_from() or explicit bounds checks instead of `as`.
// Example: u32::try_from(len).map_err(|_| Error::Overflow)?
// =============================================================================
#![allow(clippy::cast_possible_truncation)] // Can hide integer truncation bugs
#![allow(clippy::cast_precision_loss)] // Acceptable for f32/f64 conversions
#![allow(clippy::cast_possible_wrap)] // Can hide overflow bugs
#![allow(clippy::cast_sign_loss)] // Can hide sign conversion bugs
#![allow(clippy::cast_lossless)]
// Safe - just suggests Into instead of as

// =============================================================================
// STYLISTIC LINTS - Safe to allow globally (no bug risk)
// =============================================================================
#![allow(clippy::option_if_let_else)]
#![allow(clippy::significant_drop_tightening)]
#![allow(clippy::redundant_clone)]
#![allow(clippy::missing_const_for_fn)]
#![allow(clippy::suboptimal_flops)]
#![allow(clippy::derive_partial_eq_without_eq)]
#![allow(clippy::if_not_else)]
#![allow(clippy::redundant_pub_crate)]
#![allow(clippy::unused_peekable)]
#![allow(clippy::use_self)]
#![allow(clippy::significant_drop_in_scrutinee)]
#![allow(clippy::imprecise_flops)]
#![allow(clippy::set_contains_or_insert)]
#![allow(clippy::useless_let_if_seq)]
#![allow(clippy::doc_markdown)]
#![allow(clippy::single_match_else)]
#![allow(clippy::large_stack_arrays)]
#![allow(clippy::manual_let_else)]
#![allow(clippy::unused_self)]
#![allow(clippy::uninlined_format_args)]
#![allow(clippy::wildcard_imports)]
#![allow(clippy::ptr_as_ptr)]
#![allow(clippy::implicit_hasher)]
#![allow(clippy::unnecessary_cast)]
#![allow(clippy::collapsible_if)]
#![allow(clippy::used_underscore_binding)]
#![allow(clippy::manual_assert)]
#![allow(clippy::assertions_on_constants)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::unused_async)]
// =============================================================================
// THREAD SAFETY LINT - REQUIRES CAREFUL REVIEW
// =============================================================================
// FLAG-1 WARNING: This lint can hide thread safety issues. Each unsafe Send/Sync
// impl should have a // SAFETY: comment explaining why it's correct.
// See: native_inner.rs Send/Sync impl for NativeHnswInner
#![allow(clippy::non_send_fields_in_send_ty)]

#[cfg(feature = "persistence")]
pub mod agent;
pub mod alloc_guard;
#[cfg(test)]
mod alloc_guard_tests;
pub mod cache;
#[cfg(feature = "persistence")]
pub mod collection;
#[cfg(feature = "persistence")]
pub mod column_store;
#[cfg(all(test, feature = "persistence"))]
mod column_store_tests;
pub mod compression;
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
mod filter_like_tests;
#[cfg(test)]
mod filter_tests;
pub mod fusion;
pub mod gpu;
#[cfg(test)]
mod gpu_tests;
#[cfg(feature = "persistence")]
pub mod guardrails;
#[cfg(all(test, feature = "persistence"))]
mod guardrails_tests;
pub mod half_precision;
#[cfg(test)]
mod half_precision_tests;
#[cfg(feature = "persistence")]
pub mod index;
pub mod metrics;
#[cfg(test)]
mod metrics_tests;
pub mod perf_optimizations;
#[cfg(test)]
mod perf_optimizations_tests;
pub mod point;
#[cfg(test)]
mod point_tests;
pub mod quantization;
#[cfg(test)]
mod quantization_tests;
pub mod simd_dispatch;
#[cfg(test)]
mod simd_dispatch_tests;
#[cfg(test)]
mod simd_epic073_tests;
// simd_explicit removed - consolidated into simd_native (EPIC-075)
pub mod simd_native;
#[cfg(test)]
mod simd_native_tests;
#[cfg(target_arch = "aarch64")]
pub mod simd_neon;
#[cfg(target_arch = "aarch64")]
pub mod simd_neon_prefetch;
// simd_ops removed - direct dispatch via simd_native (EPIC-CLEANUP)
#[cfg(test)]
mod simd_prefetch_x86_tests;
#[cfg(test)]
mod simd_tests;
#[cfg(feature = "persistence")]
pub mod storage;
pub mod sync;
#[cfg(not(target_arch = "wasm32"))]
pub mod update_check;
pub mod vector_ref;
#[cfg(test)]
mod vector_ref_tests;
pub mod velesql;

#[cfg(all(not(target_arch = "wasm32"), feature = "update-check"))]
pub use update_check::{check_for_updates, spawn_update_check};
#[cfg(not(target_arch = "wasm32"))]
pub use update_check::{compute_instance_hash, UpdateCheckConfig};

#[cfg(feature = "persistence")]
pub use index::{HnswIndex, HnswParams, SearchQuality, VectorIndex};

#[cfg(feature = "persistence")]
pub use collection::{
    Collection, CollectionType, ConcurrentEdgeStore, EdgeStore, EdgeType, Element, GraphEdge,
    GraphNode, GraphSchema, IndexInfo, NodeType, TraversalResult, ValueType,
};
pub use distance::DistanceMetric;
pub use error::{Error, Result};
pub use filter::{Condition, Filter};
pub use point::{Point, SearchResult};
pub use quantization::{
    cosine_similarity_quantized, cosine_similarity_quantized_simd, dot_product_quantized,
    dot_product_quantized_simd, euclidean_squared_quantized, euclidean_squared_quantized_simd,
    BinaryQuantizedVector, QuantizedVector, StorageMode,
};

#[cfg(feature = "persistence")]
pub use column_store::{
    BatchUpdate, BatchUpdateResult, BatchUpsertResult, ColumnStore, ColumnStoreError, ColumnType,
    ColumnValue, ExpireResult, StringId, StringTable, TypedColumn, UpsertResult,
};
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
#[cfg(feature = "persistence")]
pub struct Database {
    /// Path to the data directory
    data_dir: std::path::PathBuf,
    /// Collections managed by this database
    collections: parking_lot::RwLock<std::collections::HashMap<String, Collection>>,
}

#[cfg(feature = "persistence")]
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

        // Log SIMD features detected at startup
        let features = simd_dispatch::simd_features_info();
        tracing::info!(
            avx512 = features.avx512f,
            avx2 = features.avx2,
            "SIMD features detected - direct dispatch enabled"
        );

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

    /// Creates a new collection with a specific type (Vector or `MetadataOnly`).
    ///
    /// # Arguments
    ///
    /// * `name` - Unique name for the collection
    /// * `collection_type` - Type of collection to create
    ///
    /// # Errors
    ///
    /// Returns an error if a collection with the same name already exists.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// use velesdb_core::{Database, CollectionType, DistanceMetric, StorageMode};
    ///
    /// let db = Database::open("./data")?;
    ///
    /// // Create a metadata-only collection
    /// db.create_collection_typed("products", CollectionType::MetadataOnly)?;
    ///
    /// // Create a vector collection
    /// db.create_collection_typed("embeddings", CollectionType::Vector {
    ///     dimension: 768,
    ///     metric: DistanceMetric::Cosine,
    ///     storage_mode: StorageMode::Full,
    /// })?;
    /// ```
    pub fn create_collection_typed(
        &self,
        name: &str,
        collection_type: &CollectionType,
    ) -> Result<()> {
        let mut collections = self.collections.write();

        if collections.contains_key(name) {
            return Err(Error::CollectionExists(name.to_string()));
        }

        let collection_path = self.data_dir.join(name);
        let collection = Collection::create_typed(collection_path, name, collection_type)?;
        collections.insert(name.to_string(), collection);

        Ok(())
    }

    /// Loads existing collections from disk.
    ///
    /// Call this after opening a database to load previously created collections.
    ///
    /// # Errors
    ///
    /// Returns an error if collection directories cannot be read.
    pub fn load_collections(&self) -> Result<()> {
        let mut collections = self.collections.write();

        for entry in std::fs::read_dir(&self.data_dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_dir() {
                let config_path = path.join("config.json");
                if config_path.exists() {
                    let name = path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("unknown")
                        .to_string();

                    if let std::collections::hash_map::Entry::Vacant(entry) =
                        collections.entry(name)
                    {
                        match Collection::open(path) {
                            Ok(collection) => {
                                entry.insert(collection);
                            }
                            Err(err) => {
                                eprintln!("Warning: Failed to load collection: {err}");
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }
}

#[cfg(all(test, feature = "persistence"))]
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
