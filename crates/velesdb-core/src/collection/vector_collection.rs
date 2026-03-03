//! `VectorCollection`: vector + payload storage with HNSW search.
//!
//! This is the primary collection type for semantic search workloads.
//! It owns a [`VectorEngine`] and a [`PayloadEngine`] and delegates
//! all heavy lifting to them.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::RwLock;

use crate::collection::types::{Collection, CollectionConfig};
use crate::distance::DistanceMetric;
use crate::engine::payload::PayloadEngine;
use crate::engine::vector::VectorEngine;
use crate::error::{Error, Result};
use crate::guardrails::GuardRails;
use crate::index::SecondaryIndex;
use crate::point::{Point, SearchResult};
use crate::quantization::StorageMode;
use crate::velesql::QueryPlanner;

/// A vector collection combining HNSW search, payload storage, and full-text search.
///
/// # Examples
///
/// ```rust,no_run
/// use velesdb_core::{VectorCollection, DistanceMetric, Point, StorageMode};
/// use serde_json::json;
///
/// let coll = VectorCollection::create(
///     "./data/docs".into(),
///     "docs",
///     768,
///     DistanceMetric::Cosine,
///     StorageMode::Full,
/// )?;
///
/// coll.upsert(vec![
///     Point::new(1, vec![0.1; 768], Some(json!({"title": "Hello"}))),
/// ])?;
///
/// let results = coll.search(&vec![0.1; 768], 10)?;
/// # Ok::<(), velesdb_core::Error>(())
/// ```
#[derive(Clone)]
pub struct VectorCollection {
    /// Path to the collection directory on disk.
    pub(crate) path: PathBuf,
    /// Collection metadata (name, dimension, metric, point_count, storage_mode).
    pub(crate) config: Arc<RwLock<CollectionConfig>>,
    /// Vector engine: HNSW index + mmap storage + quantization caches.
    pub(crate) vector: VectorEngine,
    /// Payload engine: log-structured storage + BM25 text index.
    pub(crate) payload: PayloadEngine,
    /// Secondary indexes for metadata payload fields (used by execute_query in future migration).
    #[allow(dead_code)]
    pub(crate) secondary_indexes: Arc<RwLock<HashMap<String, SecondaryIndex>>>,
    /// Guard-rails: circuit breaker + rate limiter + cardinality/timeout limits.
    #[allow(dead_code)]
    pub(crate) guard_rails: Arc<GuardRails>,
    /// Cost-based query planner (adaptive statistics).
    #[allow(dead_code)]
    pub(crate) query_planner: Arc<QueryPlanner>,
    /// Legacy executor — shared Arc with the engines above, zero extra I/O.
    /// Used by execute_query; drives guard-rails, CBO, and VelesQL dispatch.
    pub(crate) inner: Collection,
}

impl VectorCollection {
    // -------------------------------------------------------------------------
    // Lifecycle
    // -------------------------------------------------------------------------

    /// Creates a new `VectorCollection` at the given path.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created or storage initialisation fails.
    pub fn create(
        path: PathBuf,
        name: &str,
        dimension: usize,
        metric: DistanceMetric,
        storage_mode: StorageMode,
    ) -> Result<Self> {
        std::fs::create_dir_all(&path)?;

        let config = CollectionConfig {
            name: name.to_string(),
            dimension,
            metric,
            point_count: 0,
            storage_mode,
            metadata_only: false,
        };

        let self_path = path;
        let vector = VectorEngine::create(&self_path, dimension, metric, storage_mode)?;
        let payload = PayloadEngine::create(&self_path)?;

        let inner =
            Collection::create_with_options(self_path.clone(), dimension, metric, storage_mode)?;

        let coll = Self {
            path: self_path,
            config: Arc::new(RwLock::new(config)),
            vector,
            payload,
            secondary_indexes: Arc::new(RwLock::new(HashMap::new())),
            guard_rails: Arc::new(GuardRails::default()),
            query_planner: Arc::new(QueryPlanner::new()),
            inner,
        };
        Ok(coll)
    }

    /// Opens an existing `VectorCollection` from disk.
    ///
    /// # Errors
    ///
    /// Returns an error if the config file cannot be read or storage cannot be opened.
    pub fn open(path: PathBuf) -> Result<Self> {
        let config_path = path.join("config.json");
        let config_data = std::fs::read_to_string(&config_path)?;
        let config: CollectionConfig =
            serde_json::from_str(&config_data).map_err(|e| Error::Serialization(e.to_string()))?;

        let vector =
            VectorEngine::open(&path, config.dimension, config.metric, config.storage_mode)?;
        let payload = PayloadEngine::open(&path)?;
        let inner = Collection::open(path.clone())?;

        Ok(Self {
            path,
            config: Arc::new(RwLock::new(config)),
            vector,
            payload,
            secondary_indexes: Arc::new(RwLock::new(HashMap::new())),
            guard_rails: Arc::new(GuardRails::default()),
            query_planner: Arc::new(QueryPlanner::new()),
            inner,
        })
    }

    /// Saves the collection configuration to disk.
    pub(crate) fn save_config(&self) -> Result<()> {
        self.inner.save_config()
    }

    /// Flushes all engines to disk and saves the config.
    ///
    /// # Errors
    ///
    /// Returns an error if any flush operation fails.
    pub fn flush(&self) -> Result<()> {
        self.save_config()?;
        self.vector.flush(&self.path)?;
        self.payload.flush()?;
        Ok(())
    }

    // -------------------------------------------------------------------------
    // Collection metadata
    // -------------------------------------------------------------------------

    /// Returns the collection name.
    #[must_use]
    pub fn name(&self) -> String {
        self.config.read().name.clone()
    }

    /// Returns the vector dimension.
    #[must_use]
    pub fn dimension(&self) -> usize {
        self.config.read().dimension
    }

    /// Returns the distance metric.
    #[must_use]
    pub fn metric(&self) -> DistanceMetric {
        self.config.read().metric
    }

    /// Returns the storage mode.
    #[must_use]
    pub fn storage_mode(&self) -> StorageMode {
        self.config.read().storage_mode
    }

    /// Returns the number of points in the collection.
    #[must_use]
    pub fn len(&self) -> usize {
        self.config.read().point_count
    }

    /// Returns `true` if the collection is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.config.read().point_count == 0
    }

    /// Returns all point IDs.
    #[must_use]
    pub fn all_ids(&self) -> Vec<u64> {
        self.payload.ids()
    }

    /// Returns the current collection config.
    #[must_use]
    pub fn config(&self) -> CollectionConfig {
        self.config.read().clone()
    }

    // -------------------------------------------------------------------------
    // CRUD
    // -------------------------------------------------------------------------

    /// Inserts or updates points.
    ///
    /// # Errors
    ///
    /// Returns an error if the dimension mismatches or storage fails.
    pub fn upsert(&self, points: impl IntoIterator<Item = Point>) -> Result<()> {
        let points: Vec<Point> = points.into_iter().collect();
        let dimension = self.config.read().dimension;

        for point in &points {
            if point.dimension() != dimension {
                return Err(Error::DimensionMismatch {
                    expected: dimension,
                    actual: point.dimension(),
                });
            }
        }

        for point in &points {
            let old_payload = self.payload.retrieve(point.id)?;
            self.vector.store_vector(point.id, &point.vector)?;
            self.payload
                .store(point.id, point.payload.as_ref(), old_payload.as_ref())?;
        }

        let new_count = self.vector.len();
        self.config.write().point_count = new_count;
        *self.inner.cached_stats.lock() = None;
        Ok(())
    }

    /// Retrieves points by IDs.
    #[must_use]
    pub fn get(&self, ids: &[u64]) -> Vec<Option<Point>> {
        ids.iter()
            .map(|&id| {
                let vector = self.vector.retrieve_vector(id)?;
                let payload = self.payload.retrieve(id).ok().flatten();
                Some(Point {
                    id,
                    vector,
                    payload,
                })
            })
            .collect()
    }

    /// Deletes points by IDs.
    ///
    /// # Errors
    ///
    /// Returns an error if storage operations fail.
    pub fn delete(&self, ids: &[u64]) -> Result<()> {
        for &id in ids {
            self.vector.delete_vector(id);
            self.payload.delete(id)?;
        }
        let new_count = self.vector.len();
        self.config.write().point_count = new_count;
        *self.inner.cached_stats.lock() = None;
        Ok(())
    }

    // -------------------------------------------------------------------------
    // Search
    // -------------------------------------------------------------------------

    /// Performs kNN vector search.
    ///
    /// # Errors
    ///
    /// Returns an error if the query dimension mismatches.
    pub fn search(&self, query: &[f32], k: usize) -> Result<Vec<SearchResult>> {
        let dimension = self.config.read().dimension;
        if query.len() != dimension {
            return Err(Error::DimensionMismatch {
                expected: dimension,
                actual: query.len(),
            });
        }
        let ids = self.vector.search(query, k);
        let results = ids
            .into_iter()
            .filter_map(|(id, score)| {
                let vector = self.vector.retrieve_vector(id)?;
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

    /// Performs full-text BM25 search.
    #[must_use]
    pub fn text_search(&self, query: &str, k: usize) -> Vec<SearchResult> {
        self.payload
            .text_search(query, k)
            .into_iter()
            .filter_map(|(id, score)| {
                let vector = self.vector.retrieve_vector(id)?;
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
            .collect()
    }

    /// Executes a `VelesQL` query via the shared `Collection` executor.
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

    /// Executes a raw VelesQL string (uses the query cache).
    ///
    /// # Errors
    ///
    /// Returns an error if the query cannot be parsed or executed.
    pub fn execute_query_str(
        &self,
        sql: &str,
        params: &HashMap<String, serde_json::Value>,
    ) -> Result<Vec<SearchResult>> {
        let query = self
            .inner
            .query_cache
            .parse(sql)
            .map_err(|e| Error::Query(e.to_string()))?;
        self.inner.execute_query(&query, params)
    }
}
