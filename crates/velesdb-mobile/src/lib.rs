// UniFFI requires owned types (String, Vec) for FFI bindings - references not supported
#![allow(clippy::needless_pass_by_value)]
// FFI boundary - pedantic lints relaxed for UniFFI compatibility
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::must_use_candidate)]
#![allow(clippy::uninlined_format_args)]
#![allow(clippy::similar_names)]
#![allow(clippy::module_name_repetitions)]
#![allow(clippy::doc_markdown)]
#![allow(clippy::wildcard_imports)]
#![allow(clippy::redundant_closure_for_method_calls)]

//! VelesDB Mobile - Native bindings for iOS and Android
//!
//! This crate provides UniFFI bindings for VelesDB, enabling native integration
//! with Swift (iOS) and Kotlin (Android) applications.
//!
//! # Architecture
//!
//! - **iOS**: Generates Swift bindings + XCFramework (arm64 device, arm64/x86_64 simulator)
//! - **Android**: Generates Kotlin bindings + AAR (arm64-v8a, armeabi-v7a, x86_64)
//!
//! # Build Commands
//!
//! ```bash
//! # iOS
//! cargo build --release --target aarch64-apple-ios
//! cargo build --release --target aarch64-apple-ios-sim
//!
//! # Android (requires NDK)
//! cargo ndk -t arm64-v8a -t armeabi-v7a -t x86_64 build --release
//! ```

uniffi::setup_scaffolding!();

use std::sync::Arc;
use velesdb_core::collection::Collection as CoreCollection;
use velesdb_core::Database as CoreDatabase;
use velesdb_core::DistanceMetric as CoreDistanceMetric;

// ============================================================================
// Error Types
// ============================================================================

/// Errors that can occur when using VelesDB on mobile.
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum VelesError {
    /// Database operation failed.
    #[error("Database error: {message}")]
    Database { message: String },

    /// Collection operation failed.
    #[error("Collection error: {message}")]
    Collection { message: String },

    /// Vector dimension mismatch.
    #[error("Dimension mismatch: expected {expected}, got {actual}")]
    DimensionMismatch { expected: u32, actual: u32 },
}

impl From<velesdb_core::Error> for VelesError {
    fn from(err: velesdb_core::Error) -> Self {
        match err {
            velesdb_core::Error::DimensionMismatch { expected, actual } =>
            {
                #[allow(clippy::cast_possible_truncation)]
                VelesError::DimensionMismatch {
                    expected: expected as u32,
                    actual: actual as u32,
                }
            }
            velesdb_core::Error::CollectionNotFound(name) => VelesError::Collection {
                message: format!("Collection not found: {name}"),
            },
            velesdb_core::Error::CollectionExists(name) => VelesError::Collection {
                message: format!("Collection already exists: {name}"),
            },
            other => VelesError::Database {
                message: other.to_string(),
            },
        }
    }
}

// ============================================================================
// Enums
// ============================================================================

/// Distance metric for vector similarity.
#[derive(Debug, Clone, Copy, uniffi::Enum)]
pub enum DistanceMetric {
    /// Cosine similarity (1 - cosine_distance). Higher is more similar.
    Cosine,
    /// Euclidean (L2) distance. Lower is more similar.
    Euclidean,
    /// Dot product. Higher is more similar (for normalized vectors).
    DotProduct,
    /// Hamming distance for binary vectors. Lower is more similar.
    Hamming,
    /// Jaccard similarity for set-like vectors. Higher is more similar.
    Jaccard,
}

impl From<DistanceMetric> for CoreDistanceMetric {
    fn from(metric: DistanceMetric) -> Self {
        match metric {
            DistanceMetric::Cosine => CoreDistanceMetric::Cosine,
            DistanceMetric::Euclidean => CoreDistanceMetric::Euclidean,
            DistanceMetric::DotProduct => CoreDistanceMetric::DotProduct,
            DistanceMetric::Hamming => CoreDistanceMetric::Hamming,
            DistanceMetric::Jaccard => CoreDistanceMetric::Jaccard,
        }
    }
}

/// Storage mode for vector quantization (IoT/Edge optimization).
#[derive(Debug, Clone, Copy, uniffi::Enum)]
pub enum StorageMode {
    /// Full f32 precision (4 bytes/dimension). Best recall.
    Full,
    /// SQ8: 8-bit scalar quantization (1 byte/dimension). 4x compression, ~1% recall loss.
    Sq8,
    /// Binary: 1-bit quantization (1 bit/dimension). 32x compression, ~5-10% recall loss.
    Binary,
}

impl From<StorageMode> for velesdb_core::StorageMode {
    fn from(mode: StorageMode) -> Self {
        match mode {
            StorageMode::Full => velesdb_core::StorageMode::Full,
            StorageMode::Sq8 => velesdb_core::StorageMode::SQ8,
            StorageMode::Binary => velesdb_core::StorageMode::Binary,
        }
    }
}

// ============================================================================
// Data Types
// ============================================================================

/// A search result containing an ID and similarity score.
#[derive(Debug, Clone, uniffi::Record)]
pub struct SearchResult {
    /// Vector ID.
    pub id: u64,
    /// Similarity score.
    pub score: f32,
}

/// A point to insert into the database.
#[derive(Debug, Clone, uniffi::Record)]
pub struct VelesPoint {
    /// Unique identifier.
    pub id: u64,
    /// Vector embedding.
    pub vector: Vec<f32>,
    /// Optional JSON payload as string.
    pub payload: Option<String>,
}

// ============================================================================
// Database
// ============================================================================

/// VelesDB database instance.
///
/// Thread-safe handle to a VelesDB database. Can be shared across threads.
#[derive(uniffi::Object)]
pub struct VelesDatabase {
    inner: CoreDatabase,
}

#[uniffi::export]
impl VelesDatabase {
    /// Opens or creates a database at the specified path.
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the database directory (will be created if needed)
    ///
    /// # Errors
    ///
    /// Returns an error if the path is invalid or cannot be accessed.
    #[uniffi::constructor]
    pub fn open(path: String) -> Result<Arc<Self>, VelesError> {
        let db = CoreDatabase::open(&path)?;
        Ok(Arc::new(Self { inner: db }))
    }

    /// Creates a new collection with the specified parameters.
    ///
    /// # Arguments
    ///
    /// * `name` - Unique name for the collection
    /// * `dimension` - Vector dimension (e.g., 384, 768, 1536)
    /// * `metric` - Distance metric for similarity calculations
    pub fn create_collection(
        &self,
        name: String,
        dimension: u32,
        metric: DistanceMetric,
    ) -> Result<(), VelesError> {
        self.inner.create_collection(
            &name,
            usize::try_from(dimension).unwrap_or(usize::MAX),
            metric.into(),
        )?;
        Ok(())
    }

    /// Creates a new collection with custom storage mode for IoT/Edge devices.
    ///
    /// # Arguments
    ///
    /// * `name` - Unique name for the collection
    /// * `dimension` - Vector dimension
    /// * `metric` - Distance metric
    /// * `storage_mode` - Storage optimization (Full, Sq8, Binary)
    ///
    /// # Storage Modes
    ///
    /// - **Full**: Best recall, 4 bytes/dimension
    /// - **Sq8**: 4x compression, ~1% recall loss (recommended for mobile)
    /// - **Binary**: 32x compression, ~5-10% recall loss (for extreme constraints)
    pub fn create_collection_with_storage(
        &self,
        name: String,
        dimension: u32,
        metric: DistanceMetric,
        storage_mode: StorageMode,
    ) -> Result<(), VelesError> {
        self.inner.create_collection_with_options(
            &name,
            usize::try_from(dimension).unwrap_or(usize::MAX),
            metric.into(),
            storage_mode.into(),
        )?;
        Ok(())
    }

    /// Gets a collection by name.
    ///
    /// Returns `None` if the collection does not exist.
    pub fn get_collection(&self, name: String) -> Result<Option<Arc<VelesCollection>>, VelesError> {
        match self.inner.get_collection(&name) {
            Some(collection) => Ok(Some(Arc::new(VelesCollection { inner: collection }))),
            None => Ok(None),
        }
    }

    /// Lists all collection names.
    pub fn list_collections(&self) -> Vec<String> {
        self.inner.list_collections()
    }

    /// Deletes a collection by name.
    pub fn delete_collection(&self, name: String) -> Result<(), VelesError> {
        self.inner.delete_collection(&name)?;
        Ok(())
    }
}

// ============================================================================
// Collection
// ============================================================================

/// A collection of vectors with associated metadata.
#[derive(uniffi::Object)]
pub struct VelesCollection {
    inner: CoreCollection,
}

#[uniffi::export]
impl VelesCollection {
    /// Searches for the k nearest neighbors to the query vector.
    ///
    /// # Arguments
    ///
    /// * `vector` - Query vector
    /// * `limit` - Maximum number of results to return
    ///
    /// # Returns
    ///
    /// Vector of search results sorted by similarity.
    pub fn search(&self, vector: Vec<f32>, limit: u32) -> Result<Vec<SearchResult>, VelesError> {
        let results = self
            .inner
            .search_ids(&vector, usize::try_from(limit).unwrap_or(usize::MAX))?;

        Ok(results
            .into_iter()
            .map(|(id, score)| SearchResult { id, score })
            .collect())
    }

    /// Inserts or updates a single point.
    ///
    /// # Arguments
    ///
    /// * `point` - The point to upsert
    pub fn upsert(&self, point: VelesPoint) -> Result<(), VelesError> {
        let payload = point
            .payload
            .map(|s| serde_json::from_str(&s))
            .transpose()
            .map_err(|e| VelesError::Database {
                message: format!("Invalid JSON payload: {e}"),
            })?;

        let core_point = velesdb_core::Point::new(point.id, point.vector, payload);
        self.inner.upsert(vec![core_point])?;
        Ok(())
    }

    /// Inserts or updates multiple points in batch.
    ///
    /// # Arguments
    ///
    /// * `points` - Points to upsert
    pub fn upsert_batch(&self, points: Vec<VelesPoint>) -> Result<(), VelesError> {
        let core_points: Result<Vec<velesdb_core::Point>, VelesError> = points
            .into_iter()
            .map(|p| {
                let payload = p
                    .payload
                    .map(|s| serde_json::from_str(&s))
                    .transpose()
                    .map_err(|e| VelesError::Database {
                        message: format!("Invalid JSON payload: {e}"),
                    })?;
                Ok(velesdb_core::Point::new(p.id, p.vector, payload))
            })
            .collect();

        self.inner.upsert(core_points?)?;
        Ok(())
    }

    /// Deletes a point by ID.
    pub fn delete(&self, id: u64) -> Result<(), VelesError> {
        self.inner.delete(&[id])?;
        Ok(())
    }

    /// Returns the number of points in the collection.
    #[allow(clippy::cast_possible_truncation)]
    pub fn count(&self) -> u64 {
        self.inner.config().point_count as u64
    }

    /// Returns the vector dimension.
    #[allow(clippy::cast_possible_truncation)]
    pub fn dimension(&self) -> u32 {
        self.inner.config().dimension as u32
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // =========================================================================
    // DistanceMetric Tests
    // =========================================================================

    #[test]
    fn test_distance_metric_cosine_conversion() {
        let metric = DistanceMetric::Cosine;
        let core: CoreDistanceMetric = metric.into();
        assert_eq!(core, CoreDistanceMetric::Cosine);
    }

    #[test]
    fn test_distance_metric_euclidean_conversion() {
        let metric = DistanceMetric::Euclidean;
        let core: CoreDistanceMetric = metric.into();
        assert_eq!(core, CoreDistanceMetric::Euclidean);
    }

    #[test]
    fn test_distance_metric_dot_product_conversion() {
        let metric = DistanceMetric::DotProduct;
        let core: CoreDistanceMetric = metric.into();
        assert_eq!(core, CoreDistanceMetric::DotProduct);
    }

    #[test]
    fn test_distance_metric_hamming_conversion() {
        let metric = DistanceMetric::Hamming;
        let core: CoreDistanceMetric = metric.into();
        assert_eq!(core, CoreDistanceMetric::Hamming);
    }

    #[test]
    fn test_distance_metric_jaccard_conversion() {
        let metric = DistanceMetric::Jaccard;
        let core: CoreDistanceMetric = metric.into();
        assert_eq!(core, CoreDistanceMetric::Jaccard);
    }

    // =========================================================================
    // StorageMode Tests
    // =========================================================================

    #[test]
    fn test_storage_mode_full_conversion() {
        let mode = StorageMode::Full;
        let core: velesdb_core::StorageMode = mode.into();
        assert_eq!(core, velesdb_core::StorageMode::Full);
    }

    #[test]
    fn test_storage_mode_sq8_conversion() {
        let mode = StorageMode::Sq8;
        let core: velesdb_core::StorageMode = mode.into();
        assert_eq!(core, velesdb_core::StorageMode::SQ8);
    }

    #[test]
    fn test_storage_mode_binary_conversion() {
        let mode = StorageMode::Binary;
        let core: velesdb_core::StorageMode = mode.into();
        assert_eq!(core, velesdb_core::StorageMode::Binary);
    }

    // =========================================================================
    // VelesDatabase Tests
    // =========================================================================

    #[test]
    fn test_database_open_and_create_collection() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().to_str().unwrap().to_string();

        let db = VelesDatabase::open(path).unwrap();
        db.create_collection("test".to_string(), 128, DistanceMetric::Cosine)
            .unwrap();

        let collections = db.list_collections();
        assert_eq!(collections.len(), 1);
        assert_eq!(collections[0], "test");
    }

    #[test]
    fn test_database_create_collection_with_storage() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().to_str().unwrap().to_string();

        let db = VelesDatabase::open(path).unwrap();
        db.create_collection_with_storage(
            "sq8_collection".to_string(),
            384,
            DistanceMetric::Euclidean,
            StorageMode::Sq8,
        )
        .unwrap();

        let col = db.get_collection("sq8_collection".to_string()).unwrap();
        assert!(col.is_some());
        assert_eq!(col.unwrap().dimension(), 384);
    }

    #[test]
    fn test_database_delete_collection() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().to_str().unwrap().to_string();

        let db = VelesDatabase::open(path).unwrap();
        db.create_collection("to_delete".to_string(), 64, DistanceMetric::DotProduct)
            .unwrap();

        assert_eq!(db.list_collections().len(), 1);

        db.delete_collection("to_delete".to_string()).unwrap();
        assert_eq!(db.list_collections().len(), 0);
    }

    // =========================================================================
    // VelesCollection Tests
    // =========================================================================

    #[test]
    fn test_collection_upsert_and_search() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().to_str().unwrap().to_string();

        let db = VelesDatabase::open(path).unwrap();
        db.create_collection("vectors".to_string(), 4, DistanceMetric::Cosine)
            .unwrap();

        let col = db.get_collection("vectors".to_string()).unwrap().unwrap();

        // Insert a point
        let point = VelesPoint {
            id: 1,
            vector: vec![1.0, 0.0, 0.0, 0.0],
            payload: None,
        };
        col.upsert(point).unwrap();

        assert_eq!(col.count(), 1);

        // Search
        let results = col.search(vec![1.0, 0.0, 0.0, 0.0], 1).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, 1);
    }

    #[test]
    fn test_collection_upsert_batch() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().to_str().unwrap().to_string();

        let db = VelesDatabase::open(path).unwrap();
        db.create_collection("batch".to_string(), 4, DistanceMetric::Euclidean)
            .unwrap();

        let col = db.get_collection("batch".to_string()).unwrap().unwrap();

        let points = vec![
            VelesPoint {
                id: 1,
                vector: vec![1.0, 0.0, 0.0, 0.0],
                payload: None,
            },
            VelesPoint {
                id: 2,
                vector: vec![0.0, 1.0, 0.0, 0.0],
                payload: None,
            },
            VelesPoint {
                id: 3,
                vector: vec![0.0, 0.0, 1.0, 0.0],
                payload: None,
            },
        ];

        col.upsert_batch(points).unwrap();
        assert_eq!(col.count(), 3);
    }

    #[test]
    fn test_collection_delete() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().to_str().unwrap().to_string();

        let db = VelesDatabase::open(path).unwrap();
        db.create_collection("delete_test".to_string(), 4, DistanceMetric::Cosine)
            .unwrap();

        let col = db
            .get_collection("delete_test".to_string())
            .unwrap()
            .unwrap();

        col.upsert(VelesPoint {
            id: 42,
            vector: vec![1.0, 1.0, 1.0, 1.0],
            payload: None,
        })
        .unwrap();

        assert_eq!(col.count(), 1);

        col.delete(42).unwrap();
        assert_eq!(col.count(), 0);
    }

    #[test]
    fn test_collection_with_json_payload() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().to_str().unwrap().to_string();

        let db = VelesDatabase::open(path).unwrap();
        db.create_collection("with_payload".to_string(), 4, DistanceMetric::Cosine)
            .unwrap();

        let col = db
            .get_collection("with_payload".to_string())
            .unwrap()
            .unwrap();

        let point = VelesPoint {
            id: 1,
            vector: vec![0.5, 0.5, 0.5, 0.5],
            payload: Some(r#"{"title": "Hello", "category": "test"}"#.to_string()),
        };

        col.upsert(point).unwrap();
        assert_eq!(col.count(), 1);
    }

    // =========================================================================
    // All 5 Metrics Integration Tests
    // =========================================================================

    #[test]
    fn test_all_five_metrics() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().to_str().unwrap().to_string();
        let db = VelesDatabase::open(path).unwrap();

        let metrics = [
            ("cosine", DistanceMetric::Cosine),
            ("euclidean", DistanceMetric::Euclidean),
            ("dot", DistanceMetric::DotProduct),
            ("hamming", DistanceMetric::Hamming),
            ("jaccard", DistanceMetric::Jaccard),
        ];

        for (name, metric) in metrics {
            db.create_collection(name.to_string(), 4, metric).unwrap();
            let col = db.get_collection(name.to_string()).unwrap().unwrap();
            col.upsert(VelesPoint {
                id: 1,
                vector: vec![1.0, 0.0, 1.0, 0.0],
                payload: None,
            })
            .unwrap();
            assert_eq!(col.count(), 1, "Collection {name} should have 1 point");
        }

        assert_eq!(db.list_collections().len(), 5);
    }

    // =========================================================================
    // All 3 Storage Modes Integration Tests
    // =========================================================================

    #[test]
    fn test_all_three_storage_modes() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().to_str().unwrap().to_string();
        let db = VelesDatabase::open(path).unwrap();

        let modes = [
            ("full", StorageMode::Full),
            ("sq8", StorageMode::Sq8),
            ("binary", StorageMode::Binary),
        ];

        for (name, mode) in modes {
            db.create_collection_with_storage(name.to_string(), 128, DistanceMetric::Cosine, mode)
                .unwrap();

            let col = db.get_collection(name.to_string()).unwrap().unwrap();
            col.upsert(VelesPoint {
                id: 1,
                vector: vec![0.1; 128],
                payload: None,
            })
            .unwrap();
            assert_eq!(col.count(), 1, "Collection {name} should have 1 point");
        }

        assert_eq!(db.list_collections().len(), 3);
    }
}
