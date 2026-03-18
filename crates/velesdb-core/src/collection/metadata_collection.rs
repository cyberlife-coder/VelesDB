//! `MetadataCollection`: payload-only storage without vectors.
//!
//! Ideal for reference tables, catalogs, and structured metadata.
//! Supports CRUD and VelesQL queries on payload — NOT vector search.
//!
//! # Design
//!
//! `MetadataCollection` is a pure newtype over `Collection` — all operations
//! delegate to the single `inner` instance, matching the `VectorCollection` pattern
//! and eliminating any dual-storage desync risk (C-02).

use std::collections::HashMap;
use std::path::PathBuf;

use crate::collection::types::Collection;
use crate::error::{Error, Result};
use crate::point::{Point, SearchResult};

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
    /// Single source of truth — all operations delegate here (C-02 pure newtype).
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
        Ok(Self {
            inner: Collection::create_metadata_only(path, name)?,
        })
    }

    /// Opens an existing `MetadataCollection` from disk.
    ///
    /// # Errors
    ///
    /// Returns an error if config or storage cannot be opened.
    pub fn open(path: PathBuf) -> Result<Self> {
        Ok(Self {
            inner: Collection::open(path)?,
        })
    }

    /// Flushes to disk.
    ///
    /// # Errors
    ///
    /// Returns an error if the flush fails.
    pub fn flush(&self) -> Result<()> {
        self.inner.flush()
    }

    // -------------------------------------------------------------------------
    // Metadata
    // -------------------------------------------------------------------------

    /// Returns the collection name.
    #[must_use]
    pub fn name(&self) -> String {
        self.inner.config().name
    }

    /// Returns the number of items in the collection.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Returns `true` if the collection is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Returns all stored IDs.
    #[must_use]
    pub fn all_ids(&self) -> Vec<u64> {
        self.inner.all_ids()
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
        let name = self.inner.config().name;

        for point in &points {
            if !point.vector.is_empty() {
                return Err(Error::VectorNotAllowed(name.clone()));
            }
        }

        self.inner.upsert_metadata(points)
    }

    /// Retrieves items by IDs.
    #[must_use]
    pub fn get(&self, ids: &[u64]) -> Vec<Option<Point>> {
        self.inner.get(ids)
    }

    /// Deletes items by IDs.
    ///
    /// # Errors
    ///
    /// Returns an error if storage operations fail.
    pub fn delete(&self, ids: &[u64]) -> Result<()> {
        self.inner.delete(ids)
    }

    // -------------------------------------------------------------------------
    // Text search
    // -------------------------------------------------------------------------

    /// Performs BM25 full-text search over payloads.
    ///
    /// # Errors
    ///
    /// Returns an error if storage retrieval fails.
    pub fn text_search(&self, query: &str, k: usize) -> Result<Vec<SearchResult>> {
        self.inner.text_search(query, k)
    }

    // -------------------------------------------------------------------------
    // VelesQL
    // -------------------------------------------------------------------------

    /// Executes a `VelesQL` query.
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
        self.inner.execute_query_str(sql, params)
    }
}
