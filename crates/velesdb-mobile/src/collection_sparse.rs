//! Sparse vector operations for `VelesCollection` (UniFFI-exported).
//!
//! Extracted from `collection.rs` to reduce NLOC below the 500 threshold.

use velesdb_core::FusionStrategy as CoreFusionStrategy;

use crate::types::{FusionStrategy, SearchResult, VelesError, VelesPoint, VelesSparseVector};
use crate::VelesCollection;

#[uniffi::export]
impl VelesCollection {
    /// Performs sparse-only search using an inverted index.
    ///
    /// # Arguments
    ///
    /// * `sparse_vector` - Query sparse vector (parallel arrays of indices/values)
    /// * `limit` - Maximum number of results
    /// * `index_name` - Name of the sparse index (empty string for default)
    ///
    /// # Returns
    ///
    /// Vector of search results sorted by sparse similarity.
    pub fn sparse_search(
        &self,
        sparse_vector: VelesSparseVector,
        limit: u32,
        index_name: Option<String>,
    ) -> Result<Vec<SearchResult>, VelesError> {
        let core_sv = Self::to_core_sparse_vector(&sparse_vector);
        let idx_name = index_name.unwrap_or_default();

        let results = self
            .inner
            .sparse_search(
                &core_sv,
                usize::try_from(limit).unwrap_or(usize::MAX),
                &idx_name,
            )
            .map_err(|e| VelesError::Database {
                message: format!("Sparse search failed: {e}"),
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

    /// Performs hybrid dense+sparse search with RRF fusion.
    ///
    /// Combines vector similarity search with sparse (keyword) search
    /// using Reciprocal Rank Fusion.
    ///
    /// # Arguments
    ///
    /// * `vector` - Dense query vector
    /// * `sparse_vector` - Sparse query vector (parallel arrays)
    /// * `limit` - Maximum number of results
    /// * `index_name` - Name of the sparse index (empty string or `None` for default)
    ///
    /// # Returns
    ///
    /// Vector of fused search results.
    pub fn hybrid_sparse_search(
        &self,
        vector: Vec<f32>,
        sparse_vector: VelesSparseVector,
        limit: u32,
        index_name: Option<String>,
    ) -> Result<Vec<SearchResult>, VelesError> {
        let core_sv = Self::to_core_sparse_vector(&sparse_vector);
        let strategy = velesdb_core::fusion::FusionStrategy::RRF { k: 60 };
        let idx_name = index_name.unwrap_or_default();

        let results = self
            .inner
            .hybrid_sparse_search(
                &vector,
                &core_sv,
                usize::try_from(limit).unwrap_or(usize::MAX),
                &idx_name,
                &strategy,
            )
            .map_err(|e| VelesError::Database {
                message: format!("Hybrid sparse search failed: {e}"),
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

    /// Performs multi-query search with result fusion.
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
                payload: None,
            })
            .collect())
    }

    /// Performs multi-query search with metadata filtering.
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
                payload: None,
            })
            .collect())
    }

    /// Inserts or updates a point with an associated sparse vector.
    ///
    /// # Arguments
    ///
    /// * `point` - The point to upsert (dense vector + payload)
    /// * `sparse_vector` - Sparse vector to associate with this point
    pub fn upsert_with_sparse(
        &self,
        point: VelesPoint,
        sparse_vector: VelesSparseVector,
    ) -> Result<(), VelesError> {
        let payload = point
            .payload
            .map(|s| serde_json::from_str(&s))
            .transpose()
            .map_err(|e| VelesError::Database {
                message: format!("Invalid JSON payload: {e}"),
            })?;

        let core_sv = Self::to_core_sparse_vector(&sparse_vector);
        let mut sparse_map = std::collections::BTreeMap::new();
        sparse_map.insert(String::new(), core_sv);

        let core_point =
            velesdb_core::Point::with_sparse(point.id, point.vector, payload, Some(sparse_map));
        self.inner.upsert(vec![core_point])?;
        Ok(())
    }
}

impl VelesCollection {
    /// Converts a `VelesSparseVector` (UniFFI-safe parallel arrays) to the
    /// core `SparseVector` type.
    pub(crate) fn to_core_sparse_vector(
        sv: &VelesSparseVector,
    ) -> velesdb_core::sparse_index::SparseVector {
        let pairs: Vec<(u32, f32)> = sv
            .indices
            .iter()
            .copied()
            .zip(sv.values.iter().copied())
            .collect();
        velesdb_core::sparse_index::SparseVector::new(pairs)
    }
}
