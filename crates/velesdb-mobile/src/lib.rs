// Mobile SDK - pedantic/nursery lints relaxed for UniFFI FFI boundary
#![allow(clippy::pedantic)]
#![allow(clippy::nursery)]
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
//! # iOS - build for device and simulator
//! cargo build --release --target aarch64-apple-ios
//! cargo build --release --target aarch64-apple-ios-sim
//! cargo build --release --target x86_64-apple-ios  # Intel simulator
//!
//! # iOS - create universal binary + XCFramework
//! lipo -create \
//!   target/aarch64-apple-ios-sim/release/libvelesdb_mobile.a \
//!   target/x86_64-apple-ios/release/libvelesdb_mobile.a \
//!   -output target/universal-sim/libvelesdb_mobile.a
//! xcodebuild -create-xcframework \
//!   -library target/aarch64-apple-ios/release/libvelesdb_mobile.a \
//!   -library target/universal-sim/libvelesdb_mobile.a \
//!   -output VelesDB.xcframework
//!
//! # Android (requires cargo-ndk: cargo install cargo-ndk)
//! cargo ndk -t arm64-v8a -t armeabi-v7a -t x86_64 build --release
//! ```

uniffi::setup_scaffolding!();

mod agent;
mod collection;
mod collection_sparse;
mod graph;
mod query;
mod types;

pub use agent::{SemanticResult, VelesSemanticMemory};
pub use collection::VelesCollection;
pub use graph::{MobileGraphEdge, MobileGraphNode, MobileGraphStore, TraversalResult};
pub use query::{QueryResult, QueryResultKind, QueryResultRow};
pub use types::{
    DistanceMetric, FusionStrategy, IndividualSearchRequest, MobileCollectionStats,
    MobileIndexInfo, PqTrainConfig, SearchQuality, SearchResult, StorageMode, VelesError,
    VelesPoint, VelesSparseVector,
};

use std::sync::Arc;
use velesdb_core::Database as CoreDatabase;

#[cfg(test)]
use velesdb_core::DistanceMetric as CoreDistanceMetric;
#[cfg(test)]
use velesdb_core::FusionStrategy as CoreFusionStrategy;
#[cfg(test)]
use velesdb_core::SearchQuality as CoreSearchQuality;

// NOTE: VelesError, DistanceMetric, StorageMode, FusionStrategy, SearchResult,
// VelesPoint, IndividualSearchRequest moved to types.rs (EPIC-061/US-005 refactoring)
// NOTE: VelesCollection moved to collection.rs (NLOC/CC resolution)

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
    /// * `storage_mode` - Storage optimization (see [`StorageMode`])
    ///
    /// # Storage Modes
    ///
    /// - **Full**: Best recall, 4 bytes/dimension
    /// - **Sq8**: 4x compression, ~1% recall loss (recommended for mobile)
    /// - **Binary**: 32x compression, ~5-10% recall loss (for extreme constraints)
    /// - **`ProductQuantization`**: 8x-16x compression via trained codebooks
    ///   (requires a training step before upserts)
    /// - **`Rabitq`**: 32x compression with ~1-2% recall loss (1-bit with
    ///   rotation + scalar correction)
    pub fn create_collection_with_storage(
        &self,
        name: String,
        dimension: u32,
        metric: DistanceMetric,
        storage_mode: StorageMode,
    ) -> Result<(), VelesError> {
        self.inner.create_vector_collection_with_options(
            &name,
            usize::try_from(dimension).unwrap_or(usize::MAX),
            metric.into(),
            storage_mode.into(),
        )?;
        Ok(())
    }

    /// Creates a metadata-only collection (no vectors).
    ///
    /// Useful for storing reference data, lookups, or auxiliary information
    /// that doesn't require vector similarity search.
    ///
    /// # Arguments
    ///
    /// * `name` - Unique name for the collection
    pub fn create_metadata_collection(&self, name: String) -> Result<(), VelesError> {
        self.inner.create_metadata_collection(&name)?;
        Ok(())
    }

    /// Creates a graph collection for knowledge graph workloads.
    ///
    /// Creates a schemaless graph collection (no node embeddings).
    /// For graph collections with node embeddings, use
    /// [`create_graph_collection_with_embeddings`](Self::create_graph_collection_with_embeddings).
    ///
    /// # Arguments
    ///
    /// * `name` - Unique name for the collection
    pub fn create_graph_collection(&self, name: String) -> Result<(), VelesError> {
        self.inner
            .create_graph_collection(&name, velesdb_core::GraphSchema::schemaless())?;
        Ok(())
    }

    /// Creates a graph collection with node embeddings.
    ///
    /// Nodes in this collection can store vector embeddings and support
    /// similarity search alongside graph traversal.
    ///
    /// # Arguments
    ///
    /// * `name` - Unique name for the collection
    /// * `dimension` - Vector dimension for node embeddings
    /// * `metric` - Distance metric for similarity calculations
    pub fn create_graph_collection_with_embeddings(
        &self,
        name: String,
        dimension: u32,
        metric: DistanceMetric,
    ) -> Result<(), VelesError> {
        self.inner.create_graph_collection_with_embeddings(
            &name,
            velesdb_core::GraphSchema::schemaless(),
            usize::try_from(dimension).unwrap_or(usize::MAX),
            metric.into(),
        )?;
        Ok(())
    }

    /// Gets a vector collection by name.
    ///
    /// Returns `None` if the collection does not exist.
    /// Returns an error if the collection exists but is not a vector collection
    /// (use [`get_graph_collection`](Self::get_graph_collection) for graph collections).
    pub fn get_collection(&self, name: String) -> Result<Option<Arc<VelesCollection>>, VelesError> {
        match self.inner.get_any_collection(&name) {
            Some(any_coll) => {
                if !any_coll.is_vector() {
                    return Err(VelesError::Collection {
                        message: format!(
                            "Collection '{}' is not a vector collection. \
                             Use get_graph_collection() for graph collections.",
                            name
                        ),
                    });
                }
                let vc = any_coll.as_vector_collection_unchecked();
                Ok(Some(Arc::new(VelesCollection { inner: vc })))
            }
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

    /// Trains a Product Quantizer on a collection.
    ///
    /// PQ training is a database-level operation that requires access to the
    /// VelesQL TRAIN executor.
    ///
    /// # Arguments
    ///
    /// * `collection_name` - Name of the collection to train PQ on
    /// * `config` - PQ training configuration
    ///
    /// # Returns
    ///
    /// Status message from the training process.
    pub fn train_pq(
        &self,
        collection_name: String,
        config: PqTrainConfig,
    ) -> Result<String, VelesError> {
        use std::collections::HashMap;
        use velesdb_core::velesql::{Query, TrainStatement, WithValue};

        let mut params = HashMap::new();
        params.insert("m".to_string(), WithValue::Integer(i64::from(config.m)));
        params.insert("k".to_string(), WithValue::Integer(i64::from(config.k)));
        if config.opq {
            params.insert("type".to_string(), WithValue::Identifier("opq".to_string()));
        }

        let query = Query::new_train(TrainStatement {
            collection: collection_name,
            params,
        });

        let empty_params = HashMap::new();
        self.inner
            .execute_query(&query, &empty_params)
            .map_err(|e| VelesError::Database {
                message: format!("PQ training failed: {e}"),
            })?;

        Ok("PQ training complete".to_string())
    }

    /// Executes an arbitrary VelesQL query and returns structured results.
    ///
    /// This is the primary entry point for mobile apps to run the full
    /// VelesQL surface: SELECT, INSERT, UPDATE, DELETE, MATCH, DDL
    /// (CREATE/DROP/ALTER/TRUNCATE), TRAIN QUANTIZER, SHOW, DESCRIBE,
    /// EXPLAIN, ANALYZE, and FLUSH.
    ///
    /// # Arguments
    ///
    /// * `sql` - VelesQL query string
    /// * `params_json` - Optional JSON object with query parameters
    ///   (keys are bare names; use `$name` syntax in SQL).
    ///   Pass `None` or `"{}"` when no parameters are needed.
    ///
    /// # Returns
    ///
    /// A [`QueryResult`] containing the result kind, rows (as JSON strings),
    /// row count, and a human-readable status message.
    ///
    /// # Example (Swift)
    ///
    /// ```swift
    /// let result = try db.executeQuery(
    ///     sql: "SELECT * FROM docs LIMIT 10",
    ///     paramsJson: nil
    /// )
    /// for row in result.rows {
    ///     let json = try JSONSerialization.jsonObject(with: row.dataJson.data(using: .utf8)!)
    ///     print(json)
    /// }
    /// ```
    pub fn execute_query(
        &self,
        sql: String,
        params_json: Option<String>,
    ) -> Result<QueryResult, VelesError> {
        let parsed =
            velesdb_core::velesql::Parser::parse(&sql).map_err(|e| VelesError::Database {
                message: format!("VelesQL parse error: {}", e.message),
            })?;

        let params = query::parse_params(params_json)?;
        let kind = query::classify_query(&parsed);

        let core_results =
            self.inner
                .execute_query(&parsed, &params)
                .map_err(|e| VelesError::Database {
                    message: format!("Query execution failed: {e}"),
                })?;

        let rows: Result<Vec<QueryResultRow>, VelesError> =
            core_results.iter().map(query::to_result_row).collect();
        let rows = rows?;

        #[allow(clippy::cast_possible_truncation)]
        // Reason: row count from a single query will not exceed u32::MAX.
        let row_count = rows.len() as u32;
        let message = query::build_message(&kind, row_count);

        Ok(QueryResult {
            kind,
            rows,
            row_count,
            message,
        })
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
