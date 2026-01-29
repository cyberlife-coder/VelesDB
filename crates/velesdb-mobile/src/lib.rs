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
//! # iOS
//! cargo build --release --target aarch64-apple-ios
//! cargo build --release --target aarch64-apple-ios-sim
//!
//! # Android (requires NDK)
//! cargo ndk -t arm64-v8a -t armeabi-v7a -t x86_64 build --release
//! ```

uniffi::setup_scaffolding!();

mod graph;
mod types;

pub use graph::{MobileGraphEdge, MobileGraphNode, MobileGraphStore, TraversalResult};
pub use types::{
    DistanceMetric, FusionStrategy, IndividualSearchRequest, SearchResult, StorageMode, VelesError,
    VelesPoint,
};

use std::sync::Arc;
use velesdb_core::collection::Collection as CoreCollection;
use velesdb_core::Database as CoreDatabase;
use velesdb_core::FusionStrategy as CoreFusionStrategy;

#[cfg(test)]
use velesdb_core::DistanceMetric as CoreDistanceMetric;

// NOTE: VelesError, DistanceMetric, StorageMode, FusionStrategy, SearchResult,
// VelesPoint, IndividualSearchRequest moved to types.rs (EPIC-061/US-005 refactoring)

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

    /// Creates a metadata-only collection (no vectors).
    ///
    /// Useful for storing reference data, lookups, or auxiliary information
    /// that doesn't require vector similarity search.
    ///
    /// # Arguments
    ///
    /// * `name` - Unique name for the collection
    pub fn create_metadata_collection(&self, name: String) -> Result<(), VelesError> {
        use velesdb_core::CollectionType;
        self.inner
            .create_collection_typed(&name, &CollectionType::MetadataOnly)?;
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

    /// Gets points by their IDs.
    ///
    /// # Arguments
    ///
    /// * `ids` - List of point IDs to retrieve
    ///
    /// # Returns
    ///
    /// Vector of points found. Missing IDs are silently skipped.
    pub fn get(&self, ids: Vec<u64>) -> Vec<VelesPoint> {
        self.inner
            .get(&ids)
            .into_iter()
            .flatten()
            .map(|p| VelesPoint {
                id: p.id,
                vector: p.vector,
                payload: p.payload.map(|v| v.to_string()),
            })
            .collect()
    }

    /// Gets a single point by ID.
    ///
    /// # Arguments
    ///
    /// * `id` - Point ID to retrieve
    ///
    /// # Returns
    ///
    /// The point if found, None otherwise.
    pub fn get_by_id(&self, id: u64) -> Option<VelesPoint> {
        self.inner
            .get(&[id])
            .into_iter()
            .flatten()
            .next()
            .map(|p| VelesPoint {
                id: p.id,
                vector: p.vector,
                payload: p.payload.map(|v| v.to_string()),
            })
    }

    /// Checks if this is a metadata-only collection.
    pub fn is_metadata_only(&self) -> bool {
        self.inner.config().metadata_only
    }

    /// Performs full-text search using BM25.
    ///
    /// # Arguments
    ///
    /// * `query` - Text query to search for
    /// * `limit` - Maximum number of results to return
    ///
    /// # Returns
    ///
    /// Vector of search results sorted by BM25 score.
    pub fn text_search(&self, query: String, limit: u32) -> Vec<SearchResult> {
        let results = self
            .inner
            .text_search(&query, usize::try_from(limit).unwrap_or(usize::MAX));

        results
            .into_iter()
            .map(|r| SearchResult {
                id: r.point.id,
                score: r.score,
            })
            .collect()
    }

    /// Performs hybrid search combining vector similarity and BM25 text search.
    ///
    /// # Arguments
    ///
    /// * `vector` - Query vector for similarity search
    /// * `text_query` - Text query for BM25 search
    /// * `limit` - Maximum number of results
    /// * `vector_weight` - Weight for vector similarity (0.0-1.0)
    ///
    /// # Returns
    ///
    /// Vector of search results sorted by fused score.
    pub fn hybrid_search(
        &self,
        vector: Vec<f32>,
        text_query: String,
        limit: u32,
        vector_weight: f32,
    ) -> Result<Vec<SearchResult>, VelesError> {
        let results = self.inner.hybrid_search(
            &vector,
            &text_query,
            usize::try_from(limit).unwrap_or(usize::MAX),
            Some(vector_weight),
        )?;

        Ok(results
            .into_iter()
            .map(|r| SearchResult {
                id: r.point.id,
                score: r.score,
            })
            .collect())
    }

    /// Searches with metadata filtering.
    ///
    /// # Arguments
    ///
    /// * `vector` - Query vector
    /// * `limit` - Maximum number of results
    /// * `filter_json` - JSON filter string (e.g., `{"condition": {"type": "eq", "field": "category", "value": "tech"}}`)
    ///
    /// # Returns
    ///
    /// Vector of search results matching the filter.
    pub fn search_with_filter(
        &self,
        vector: Vec<f32>,
        limit: u32,
        filter_json: String,
    ) -> Result<Vec<SearchResult>, VelesError> {
        // Parse filter JSON
        let filter: velesdb_core::Filter =
            serde_json::from_str(&filter_json).map_err(|e| VelesError::Database {
                message: format!("Invalid filter JSON: {e}"),
            })?;

        let results = self.inner.search_with_filter(
            &vector,
            usize::try_from(limit).unwrap_or(usize::MAX),
            &filter,
        )?;

        Ok(results
            .into_iter()
            .map(|r| SearchResult {
                id: r.point.id,
                score: r.score,
            })
            .collect())
    }

    /// Performs batch search for multiple query vectors in parallel.
    ///
    /// # Arguments
    ///
    /// * `searches` - List of search requests
    ///
    /// # Returns
    ///
    /// List of result lists (one per query vector).
    pub fn batch_search(
        &self,
        searches: Vec<IndividualSearchRequest>,
    ) -> Result<Vec<Vec<SearchResult>>, VelesError> {
        let query_refs: Vec<&[f32]> = searches.iter().map(|s| s.vector.as_slice()).collect();

        let filters: Result<Vec<Option<velesdb_core::Filter>>, VelesError> = searches
            .iter()
            .map(|s| {
                s.filter
                    .as_ref()
                    .map(|f_json| {
                        serde_json::from_str(f_json).map_err(|e| VelesError::Database {
                            message: format!("Invalid filter JSON in batch: {e}"),
                        })
                    })
                    .transpose()
            })
            .collect();

        let filters = filters?;
        let max_top_k = searches.iter().map(|s| s.top_k).max().unwrap_or(10);

        let all_results = self.inner.search_batch_with_filters(
            &query_refs,
            usize::try_from(max_top_k).unwrap_or(usize::MAX),
            &filters,
        )?;

        Ok(all_results
            .into_iter()
            .zip(searches)
            .map(
                |(results, s): (Vec<velesdb_core::SearchResult>, IndividualSearchRequest)| {
                    results
                        .into_iter()
                        .take(usize::try_from(s.top_k).unwrap_or(usize::MAX))
                        .map(|r| SearchResult {
                            id: r.point.id,
                            score: r.score,
                        })
                        .collect()
                },
            )
            .collect())
    }

    /// Performs text search with metadata filtering.
    ///
    /// # Arguments
    ///
    /// * `query` - Text query
    /// * `limit` - Maximum number of results
    /// * `filter_json` - JSON filter string
    pub fn text_search_with_filter(
        &self,
        query: String,
        limit: u32,
        filter_json: String,
    ) -> Result<Vec<SearchResult>, VelesError> {
        let filter: velesdb_core::Filter =
            serde_json::from_str(&filter_json).map_err(|e| VelesError::Database {
                message: format!("Invalid filter JSON: {e}"),
            })?;

        let results = self.inner.text_search_with_filter(
            &query,
            usize::try_from(limit).unwrap_or(usize::MAX),
            &filter,
        );

        Ok(results
            .into_iter()
            .map(|r| SearchResult {
                id: r.point.id,
                score: r.score,
            })
            .collect())
    }

    /// Performs hybrid search with metadata filtering.
    ///
    /// # Arguments
    ///
    /// * `vector` - Query vector
    /// * `text_query` - Text query
    /// * `limit` - Maximum number of results
    /// * `vector_weight` - Weight for vector similarity (0.0-1.0)
    /// * `filter_json` - JSON filter string
    pub fn hybrid_search_with_filter(
        &self,
        vector: Vec<f32>,
        text_query: String,
        limit: u32,
        vector_weight: f32,
        filter_json: String,
    ) -> Result<Vec<SearchResult>, VelesError> {
        let filter: velesdb_core::Filter =
            serde_json::from_str(&filter_json).map_err(|e| VelesError::Database {
                message: format!("Invalid filter JSON: {e}"),
            })?;

        let results = self.inner.hybrid_search_with_filter(
            &vector,
            &text_query,
            usize::try_from(limit).unwrap_or(usize::MAX),
            Some(vector_weight),
            &filter,
        )?;

        Ok(results
            .into_iter()
            .map(|r| SearchResult {
                id: r.point.id,
                score: r.score,
            })
            .collect())
    }

    /// Executes a VelesQL query.
    ///
    /// # Arguments
    ///
    /// * `query_str` - VelesQL query string
    /// * `params_json` - Optional JSON object with query parameters
    ///
    /// # Returns
    ///
    /// Vector of search results.
    ///
    /// # Example
    ///
    /// ```swift
    /// let results = try collection.query(
    ///     "SELECT * FROM vectors WHERE category = 'tech' LIMIT 10",
    ///     nil
    /// )
    /// ```
    pub fn query(
        &self,
        query_str: String,
        params_json: Option<String>,
    ) -> Result<Vec<SearchResult>, VelesError> {
        // Parse the VelesQL query
        let parsed =
            velesdb_core::velesql::Parser::parse(&query_str).map_err(|e| VelesError::Database {
                message: format!("VelesQL parse error: {}", e.message),
            })?;

        // Parse params from JSON if provided
        let params: std::collections::HashMap<String, serde_json::Value> = params_json
            .map(|json| serde_json::from_str(&json))
            .transpose()
            .map_err(|e| VelesError::Database {
                message: format!("Invalid params JSON: {e}"),
            })?
            .unwrap_or_default();

        // Execute the query
        let results =
            self.inner
                .execute_query(&parsed, &params)
                .map_err(|e| VelesError::Database {
                    message: format!("Query execution failed: {e}"),
                })?;

        Ok(results
            .into_iter()
            .map(|r| SearchResult {
                id: r.point.id,
                score: r.score,
            })
            .collect())
    }

    /// Performs multi-query search with result fusion.
    ///
    /// Executes parallel searches for multiple query vectors and fuses
    /// results using the specified strategy. Ideal for Multiple Query
    /// Generation (MQG) pipelines on mobile.
    ///
    /// # Arguments
    ///
    /// * `vectors` - List of query vectors
    /// * `limit` - Maximum number of results after fusion
    /// * `strategy` - Fusion strategy to use
    ///
    /// # Returns
    ///
    /// Vector of fused search results sorted by relevance.
    ///
    /// # Example
    ///
    /// ```swift
    /// let results = try collection.multiQuerySearch(
    ///     vectors: [query1, query2, query3],
    ///     limit: 10,
    ///     strategy: .rrf(k: 60)
    /// )
    /// ```
    pub fn multi_query_search(
        &self,
        vectors: Vec<Vec<f32>>,
        limit: u32,
        strategy: FusionStrategy,
    ) -> Result<Vec<SearchResult>, VelesError> {
        if vectors.is_empty() {
            return Err(VelesError::Database {
                message: "multi_query_search requires at least one vector".to_string(),
            });
        }

        let query_refs: Vec<&[f32]> = vectors.iter().map(|v| v.as_slice()).collect();
        let core_strategy: CoreFusionStrategy = strategy.into();

        let results = self
            .inner
            .multi_query_search(
                &query_refs,
                usize::try_from(limit).unwrap_or(usize::MAX),
                core_strategy,
                None,
            )
            .map_err(|e| VelesError::Database {
                message: format!("Multi-query search failed: {e}"),
            })?;

        Ok(results
            .into_iter()
            .map(|r| SearchResult {
                id: r.point.id,
                score: r.score,
            })
            .collect())
    }

    /// Performs multi-query search with metadata filtering.
    ///
    /// # Arguments
    ///
    /// * `vectors` - List of query vectors
    /// * `limit` - Maximum number of results after fusion
    /// * `strategy` - Fusion strategy to use
    /// * `filter_json` - JSON filter string
    pub fn multi_query_search_with_filter(
        &self,
        vectors: Vec<Vec<f32>>,
        limit: u32,
        strategy: FusionStrategy,
        filter_json: String,
    ) -> Result<Vec<SearchResult>, VelesError> {
        if vectors.is_empty() {
            return Err(VelesError::Database {
                message: "multi_query_search requires at least one vector".to_string(),
            });
        }

        let filter: velesdb_core::Filter =
            serde_json::from_str(&filter_json).map_err(|e| VelesError::Database {
                message: format!("Invalid filter JSON: {e}"),
            })?;

        let query_refs: Vec<&[f32]> = vectors.iter().map(|v| v.as_slice()).collect();
        let core_strategy: CoreFusionStrategy = strategy.into();

        let results = self
            .inner
            .multi_query_search(
                &query_refs,
                usize::try_from(limit).unwrap_or(usize::MAX),
                core_strategy,
                Some(&filter),
            )
            .map_err(|e| VelesError::Database {
                message: format!("Multi-query search failed: {e}"),
            })?;

        Ok(results
            .into_iter()
            .map(|r| SearchResult {
                id: r.point.id,
                score: r.score,
            })
            .collect())
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
