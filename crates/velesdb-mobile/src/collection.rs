//! `VelesCollection` — UniFFI-exported collection operations for mobile.

use velesdb_core::VectorCollection as CoreCollection;

use crate::types::{
    IndividualSearchRequest, MobileCollectionStats, MobileIndexInfo, SearchResult, VelesError,
    VelesPoint,
};

// ============================================================================
// Collection
// ============================================================================

/// A collection of vectors with associated metadata.
#[derive(uniffi::Object)]
pub struct VelesCollection {
    pub(crate) inner: CoreCollection,
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
            .map(|sr| SearchResult {
                id: sr.id,
                score: sr.score,
                payload: None,
            })
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
    pub fn text_search(&self, query: String, limit: u32) -> Result<Vec<SearchResult>, VelesError> {
        let results = self
            .inner
            .text_search(&query, usize::try_from(limit).unwrap_or(usize::MAX))
            .map_err(|e| VelesError::Database {
                message: format!("Text search failed: {e}"),
            })?;

        Ok(results
            .into_iter()
            .map(|r| SearchResult {
                id: r.point.id,
                score: r.score,
                payload: None,
            })
            .collect())
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
                payload: None,
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
                payload: None,
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
                            payload: None,
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

        let results = self
            .inner
            .text_search_with_filter(
                &query,
                usize::try_from(limit).unwrap_or(usize::MAX),
                &filter,
            )
            .map_err(|e| VelesError::Database {
                message: format!("Text search with filter failed: {e}"),
            })?;

        Ok(results
            .into_iter()
            .map(|r| SearchResult {
                id: r.point.id,
                score: r.score,
                payload: None,
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
                payload: None,
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
                payload: r.point.payload.as_ref().map(|p| p.to_string()),
            })
            .collect())
    }

    // multi_query_search and multi_query_search_with_filter are in collection_sparse.rs

    /// Flushes collection data to durable storage.
    pub fn flush(&self) -> Result<(), VelesError> {
        self.inner.flush()?;
        Ok(())
    }

    /// Returns all point IDs currently present in the collection.
    pub fn all_ids(&self) -> Vec<u64> {
        self.inner.all_ids()
    }

    /// Creates a secondary metadata index for a payload field.
    pub fn create_index(&self, field_name: String) -> Result<(), VelesError> {
        self.inner.create_index(&field_name)?;
        Ok(())
    }

    /// Checks whether a secondary metadata index exists for a field.
    pub fn has_secondary_index(&self, field_name: String) -> bool {
        self.inner.has_secondary_index(&field_name)
    }

    /// Creates a graph/property index for equality lookups.
    pub fn create_property_index(&self, label: String, property: String) -> Result<(), VelesError> {
        self.inner.create_property_index(&label, &property)?;
        Ok(())
    }

    /// Creates a graph/range index for range queries.
    pub fn create_range_index(&self, label: String, property: String) -> Result<(), VelesError> {
        self.inner.create_range_index(&label, &property)?;
        Ok(())
    }

    /// Checks if a property index exists.
    pub fn has_property_index(&self, label: String, property: String) -> bool {
        self.inner.has_property_index(&label, &property)
    }

    /// Checks if a range index exists.
    pub fn has_range_index(&self, label: String, property: String) -> bool {
        self.inner.has_range_index(&label, &property)
    }

    /// Lists all index definitions on this collection.
    pub fn list_indexes(&self) -> Vec<MobileIndexInfo> {
        self.inner
            .list_indexes()
            .into_iter()
            .map(MobileIndexInfo::from)
            .collect()
    }

    /// Drops an index and returns true when something was removed.
    pub fn drop_index(&self, label: String, property: String) -> Result<bool, VelesError> {
        Ok(self.inner.drop_index(&label, &property)?)
    }

    /// Returns total memory usage used by indexes.
    pub fn indexes_memory_usage(&self) -> u64 {
        u64::try_from(self.inner.indexes_memory_usage()).unwrap_or(u64::MAX)
    }

    /// Runs ANALYZE and returns fresh statistics for this collection.
    pub fn analyze(&self) -> Result<MobileCollectionStats, VelesError> {
        Ok(self.inner.analyze()?.into())
    }

    /// Returns the latest known collection statistics snapshot.
    pub fn get_stats(&self) -> MobileCollectionStats {
        self.inner.get_stats().into()
    }
}

// Sparse vector operations are in collection_sparse.rs
