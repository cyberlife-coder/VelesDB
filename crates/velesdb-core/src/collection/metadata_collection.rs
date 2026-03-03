//! `MetadataCollection`: payload-only storage without vectors.
//!
//! Ideal for reference tables, catalogs, and structured metadata.
//! Supports CRUD and VelesQL queries on payload — NOT vector search.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::RwLock;

use crate::collection::types::{Collection, CollectionConfig};
use crate::distance::DistanceMetric;
use crate::engine::payload::PayloadEngine;
use crate::error::{Error, Result};
use crate::guardrails::GuardRails;
use crate::point::{Point, SearchResult};
use crate::quantization::StorageMode;
use crate::velesql::QueryPlanner;

/// A metadata-only collection storing structured payloads without vector indexes.
///
/// # Examples
///
/// ```rust,no_run
/// use velesdb_core::{MetadataCollection, Point};
/// use serde_json::json;
///
/// let coll = MetadataCollection::create("./data/products".into(), "products")?;
///
/// coll.upsert(vec![
///     Point::metadata_only(1, json!({"name": "Widget", "price": 9.99})),
/// ])?;
/// # Ok::<(), velesdb_core::Error>(())
/// ```
#[derive(Clone)]
pub struct MetadataCollection {
    /// Path to the collection directory.
    pub(crate) path: PathBuf,
    /// Collection metadata.
    pub(crate) config: Arc<RwLock<CollectionConfig>>,
    /// Payload engine (only engine — no vectors).
    pub(crate) payload: PayloadEngine,
    /// Guard-rails (reserved for future native executor).
    #[allow(dead_code)]
    pub(crate) guard_rails: Arc<GuardRails>,
    /// Query planner (reserved for future native executor).
    #[allow(dead_code)]
    pub(crate) query_planner: Arc<QueryPlanner>,
    /// Shared executor for VelesQL queries.
    pub(crate) inner: Collection,
}

impl MetadataCollection {
    // -------------------------------------------------------------------------
    // Lifecycle
    // -------------------------------------------------------------------------

    /// Creates a new `MetadataCollection`.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created or storage fails.
    pub fn create(path: PathBuf, name: &str) -> Result<Self> {
        std::fs::create_dir_all(&path)?;

        let config = CollectionConfig {
            name: name.to_string(),
            dimension: 0,
            metric: DistanceMetric::Cosine, // Unused for metadata collections
            point_count: 0,
            storage_mode: StorageMode::Full,
            metadata_only: true,
        };

        let payload = PayloadEngine::create(&path)?;
        let inner = Collection::create_metadata_only(path.clone(), name)?;

        let coll = Self {
            path,
            config: Arc::new(RwLock::new(config)),
            payload,
            guard_rails: Arc::new(GuardRails::default()),
            query_planner: Arc::new(QueryPlanner::new()),
            inner,
        };
        Ok(coll)
    }

    /// Opens an existing `MetadataCollection` from disk.
    ///
    /// # Errors
    ///
    /// Returns an error if config or storage cannot be opened.
    pub fn open(path: PathBuf) -> Result<Self> {
        let config_path = path.join("config.json");
        let config_data = std::fs::read_to_string(&config_path)?;
        let config: CollectionConfig =
            serde_json::from_str(&config_data).map_err(|e| Error::Serialization(e.to_string()))?;

        let payload = PayloadEngine::open(&path)?;
        let inner = Collection::open(path.clone())?;

        Ok(Self {
            path,
            config: Arc::new(RwLock::new(config)),
            payload,
            guard_rails: Arc::new(GuardRails::default()),
            query_planner: Arc::new(QueryPlanner::new()),
            inner,
        })
    }

    pub(crate) fn save_config(&self) -> Result<()> {
        self.inner.save_config()
    }

    /// Flushes the payload engine to disk.
    ///
    /// # Errors
    ///
    /// Returns an error if the flush fails.
    pub fn flush(&self) -> Result<()> {
        self.save_config()?;
        self.payload.flush()
    }

    // -------------------------------------------------------------------------
    // Metadata
    // -------------------------------------------------------------------------

    /// Returns the collection name.
    #[must_use]
    pub fn name(&self) -> String {
        self.config.read().name.clone()
    }

    /// Returns the number of items in the collection.
    #[must_use]
    pub fn len(&self) -> usize {
        self.config.read().point_count
    }

    /// Returns `true` if the collection is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.config.read().point_count == 0
    }

    /// Returns all stored IDs.
    #[must_use]
    pub fn all_ids(&self) -> Vec<u64> {
        self.payload.ids()
    }

    // -------------------------------------------------------------------------
    // CRUD
    // -------------------------------------------------------------------------

    /// Inserts or updates metadata points (must have no vector).
    ///
    /// # Errors
    ///
    /// Returns an error if a point carries a non-empty vector,
    /// or if storage operations fail.
    pub fn upsert(&self, points: impl IntoIterator<Item = Point>) -> Result<()> {
        let points: Vec<Point> = points.into_iter().collect();
        let name = self.config.read().name.clone();

        for point in &points {
            if !point.vector.is_empty() {
                return Err(Error::VectorNotAllowed(name.clone()));
            }
        }

        for point in &points {
            let old = self.payload.retrieve(point.id)?;
            self.payload
                .store(point.id, point.payload.as_ref(), old.as_ref())?;
        }

        let new_count = self.payload.len();
        self.config.write().point_count = new_count;
        Ok(())
    }

    /// Retrieves items by IDs.
    #[must_use]
    pub fn get(&self, ids: &[u64]) -> Vec<Option<Point>> {
        ids.iter()
            .map(|&id| {
                let payload = self.payload.retrieve(id).ok().flatten()?;
                Some(Point {
                    id,
                    vector: Vec::new(),
                    payload: Some(payload),
                })
            })
            .collect()
    }

    /// Deletes items by IDs.
    ///
    /// # Errors
    ///
    /// Returns an error if storage operations fail.
    pub fn delete(&self, ids: &[u64]) -> Result<()> {
        for &id in ids {
            self.payload.delete(id)?;
        }
        let new_count = self.payload.len();
        self.config.write().point_count = new_count;
        Ok(())
    }

    // -------------------------------------------------------------------------
    // Text search
    // -------------------------------------------------------------------------

    /// Performs BM25 full-text search over payloads.
    #[must_use]
    pub fn text_search(&self, query: &str, k: usize) -> Vec<SearchResult> {
        self.payload
            .text_search(query, k)
            .into_iter()
            .filter_map(|(id, score)| {
                let payload = self.payload.retrieve(id).ok().flatten()?;
                Some(SearchResult::new(
                    Point {
                        id,
                        vector: Vec::new(),
                        payload: Some(payload),
                    },
                    score,
                ))
            })
            .collect()
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
        let legacy = crate::collection::Collection::open(self.path.clone())?;
        legacy.execute_query(query, params)
    }

    /// Executes a raw VelesQL string.
    ///
    /// # Errors
    ///
    /// Returns an error if parsing or execution fails.
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
