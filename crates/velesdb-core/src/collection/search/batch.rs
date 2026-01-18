//! Batch and multi-query search methods for Collection.

use crate::collection::types::Collection;
use crate::error::{Error, Result};
use crate::index::SearchQuality;
use crate::point::{Point, SearchResult};
use crate::storage::{PayloadStorage, VectorStorage};

impl Collection {
    /// Performs batch search for multiple query vectors in parallel with metadata filtering.
    /// Supports a different filter for each query in the batch.
    ///
    /// # Arguments
    ///
    /// * `queries` - List of query vector slices
    /// * `k` - Maximum number of results per query
    /// * `filters` - List of optional filters (must match queries length)
    ///
    /// # Returns
    ///
    /// Vector of search results for each query, matching its respective filter.
    ///
    /// # Errors
    ///
    /// Returns an error if queries and filters have different lengths or dimension mismatch.
    pub fn search_batch_with_filters(
        &self,
        queries: &[&[f32]],
        k: usize,
        filters: &[Option<crate::filter::Filter>],
    ) -> Result<Vec<Vec<SearchResult>>> {
        if queries.len() != filters.len() {
            return Err(Error::Config(format!(
                "Queries count ({}) does not match filters count ({})",
                queries.len(),
                filters.len()
            )));
        }

        let config = self.config.read();
        let dimension = config.dimension;
        drop(config);

        // Validate all query dimensions
        for query in queries {
            if query.len() != dimension {
                return Err(Error::DimensionMismatch {
                    expected: dimension,
                    actual: query.len(),
                });
            }
        }

        // We need to retrieve more candidates for post-filtering
        let candidates_k = k.saturating_mul(4).max(k + 10);
        let index_results =
            self.index
                .search_batch_parallel(queries, candidates_k, SearchQuality::Balanced);

        let vector_storage = self.vector_storage.read();
        let payload_storage = self.payload_storage.read();

        let mut all_results = Vec::with_capacity(queries.len());

        for (query_results, filter_opt) in index_results.into_iter().zip(filters) {
            let mut filtered_results: Vec<SearchResult> = query_results
                .into_iter()
                .filter_map(|(id, score)| {
                    let payload = payload_storage.retrieve(id).ok().flatten();

                    // Apply filter if present
                    if let Some(ref filter) = filter_opt {
                        if let Some(ref p) = payload {
                            if !filter.matches(p) {
                                return None;
                            }
                        } else if !filter.matches(&serde_json::Value::Null) {
                            return None;
                        }
                    }

                    let vector = vector_storage.retrieve(id).ok().flatten()?;
                    Some(SearchResult {
                        point: Point {
                            id,
                            vector,
                            payload,
                        },
                        score,
                    })
                })
                .collect();

            // Sort and truncate to k
            let higher_is_better = self.config.read().metric.higher_is_better();
            if higher_is_better {
                filtered_results.sort_by(|a, b| {
                    b.score
                        .partial_cmp(&a.score)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
            } else {
                filtered_results.sort_by(|a, b| {
                    a.score
                        .partial_cmp(&b.score)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
            }
            filtered_results.truncate(k);

            all_results.push(filtered_results);
        }

        Ok(all_results)
    }

    /// Performs batch search for multiple query vectors in parallel with a single metadata filter.
    ///
    /// # Arguments
    ///
    /// * `queries` - List of query vector slices
    /// * `k` - Maximum number of results per query
    /// * `filter` - Metadata filter to apply to all results
    ///
    /// # Errors
    ///
    /// Returns an error if any query has incorrect dimension.
    pub fn search_batch_with_filter(
        &self,
        queries: &[&[f32]],
        k: usize,
        filter: &crate::filter::Filter,
    ) -> Result<Vec<Vec<SearchResult>>> {
        let filters: Vec<Option<crate::filter::Filter>> = vec![Some(filter.clone()); queries.len()];
        self.search_batch_with_filters(queries, k, &filters)
    }

    /// Performs batch search for multiple query vectors in parallel.
    ///
    /// This method is optimized for high throughput using parallel index traversal.
    ///
    /// # Arguments
    ///
    /// * `queries` - List of query vector slices
    /// * `k` - Maximum number of results per query
    ///
    /// # Returns
    ///
    /// Vector of search results for each query, with full point data.
    ///
    /// # Errors
    ///
    /// Returns an error if any query vector dimension doesn't match the collection.
    pub fn search_batch_parallel(
        &self,
        queries: &[&[f32]],
        k: usize,
    ) -> Result<Vec<Vec<SearchResult>>> {
        let config = self.config.read();
        let dimension = config.dimension;
        drop(config);

        // Validate all query dimensions first
        for query in queries {
            if query.len() != dimension {
                return Err(Error::DimensionMismatch {
                    expected: dimension,
                    actual: query.len(),
                });
            }
        }

        // Perf: Use parallel HNSW search (P0 optimization)
        let index_results = self
            .index
            .search_batch_parallel(queries, k, SearchQuality::Balanced);

        // Map results to SearchResult with full point data
        let vector_storage = self.vector_storage.read();
        let payload_storage = self.payload_storage.read();

        let results: Vec<Vec<SearchResult>> = index_results
            .into_iter()
            .map(|query_results: Vec<(u64, f32)>| {
                query_results
                    .into_iter()
                    .filter_map(|(id, score)| {
                        let vector = vector_storage.retrieve(id).ok().flatten()?;
                        let payload = payload_storage.retrieve(id).ok().flatten();
                        Some(SearchResult {
                            point: Point {
                                id,
                                vector,
                                payload,
                            },
                            score,
                        })
                    })
                    .collect()
            })
            .collect();

        Ok(results)
    }

    /// Performs multi-query search with result fusion.
    ///
    /// This method executes parallel searches for multiple query vectors and fuses
    /// the results using the specified fusion strategy. Ideal for Multiple Query
    /// Generation (MQG) pipelines where multiple reformulations of a user query
    /// are searched simultaneously.
    ///
    /// # Arguments
    ///
    /// * `vectors` - Slice of query vectors (all must have same dimension)
    /// * `top_k` - Maximum number of results to return after fusion
    /// * `fusion` - Strategy for combining results (Average, Maximum, RRF, Weighted)
    /// * `filter` - Optional metadata filter to apply to all queries
    ///
    /// # Returns
    ///
    /// Vector of `SearchResult` sorted by fused score descending.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - `vectors` is empty
    /// - Any vector has incorrect dimension
    /// - More than 10 vectors are provided (configurable limit)
    #[allow(clippy::needless_pass_by_value)]
    pub fn multi_query_search(
        &self,
        vectors: &[&[f32]],
        top_k: usize,
        fusion: crate::fusion::FusionStrategy,
        filter: Option<&crate::filter::Filter>,
    ) -> Result<Vec<SearchResult>> {
        const MAX_VECTORS: usize = 10;

        // Validation: non-empty
        if vectors.is_empty() {
            return Err(Error::Config(
                "multi_query_search requires at least one vector".into(),
            ));
        }

        // Validation: max vectors limit
        if vectors.len() > MAX_VECTORS {
            return Err(Error::Config(format!(
                "multi_query_search supports at most {MAX_VECTORS} vectors, got {}",
                vectors.len()
            )));
        }

        // Validation: dimension consistency
        let config = self.config.read();
        let dimension = config.dimension;
        drop(config);

        for vector in vectors {
            if vector.len() != dimension {
                return Err(Error::DimensionMismatch {
                    expected: dimension,
                    actual: vector.len(),
                });
            }
        }

        // Calculate overfetch factor for better fusion quality
        let overfetch_k = match top_k {
            0..=10 => top_k * 20,
            11..=50 => top_k * 10,
            51..=100 => top_k * 5,
            _ => top_k * 2,
        };

        // Execute parallel batch search
        let batch_results =
            self.index
                .search_batch_parallel(vectors, overfetch_k, crate::SearchQuality::Balanced);

        // Apply filter if present (pre-fusion filtering)
        let filtered_results: Vec<Vec<(u64, f32)>> = if let Some(f) = filter {
            let payload_storage = self.payload_storage.read();
            batch_results
                .into_iter()
                .map(|query_results| {
                    query_results
                        .into_iter()
                        .filter(|(id, _score)| {
                            if let Ok(Some(payload)) = payload_storage.retrieve(*id) {
                                f.matches(&payload)
                            } else {
                                false
                            }
                        })
                        .collect()
                })
                .collect()
        } else {
            batch_results
        };

        // Fuse results using the specified strategy
        let fused = fusion
            .fuse(filtered_results)
            .map_err(|e| Error::Config(format!("Fusion error: {e}")))?;

        // Fetch full point data for top_k results
        let vector_storage = self.vector_storage.read();
        let payload_storage = self.payload_storage.read();

        let results: Vec<SearchResult> = fused
            .into_iter()
            .take(top_k)
            .filter_map(|(id, score)| {
                let vector = vector_storage.retrieve(id).ok().flatten()?;
                let payload = payload_storage.retrieve(id).ok().flatten();

                let point = Point {
                    id,
                    vector,
                    payload,
                };

                Some(SearchResult::new(point, score))
            })
            .collect();

        Ok(results)
    }

    /// Performs multi-query search returning only IDs and fused scores.
    ///
    /// This is a faster variant of `multi_query_search` that skips fetching
    /// vector and payload data. Use when you only need document IDs.
    ///
    /// # Arguments
    ///
    /// * `vectors` - Slice of query vectors
    /// * `top_k` - Maximum number of results
    /// * `fusion` - Fusion strategy
    ///
    /// # Returns
    ///
    /// Vector of `(id, fused_score)` tuples sorted by score descending.
    ///
    /// # Errors
    ///
    /// Returns an error if vectors is empty, exceeds max limit, or has dimension mismatch.
    #[allow(clippy::needless_pass_by_value)]
    pub fn multi_query_search_ids(
        &self,
        vectors: &[&[f32]],
        top_k: usize,
        fusion: crate::fusion::FusionStrategy,
    ) -> Result<Vec<(u64, f32)>> {
        const MAX_VECTORS: usize = 10;

        if vectors.is_empty() {
            return Err(Error::Config(
                "multi_query_search requires at least one vector".into(),
            ));
        }

        if vectors.len() > MAX_VECTORS {
            return Err(Error::Config(format!(
                "multi_query_search supports at most {MAX_VECTORS} vectors, got {}",
                vectors.len()
            )));
        }

        let config = self.config.read();
        let dimension = config.dimension;
        drop(config);

        for vector in vectors {
            if vector.len() != dimension {
                return Err(Error::DimensionMismatch {
                    expected: dimension,
                    actual: vector.len(),
                });
            }
        }

        let overfetch_k = match top_k {
            0..=10 => top_k * 20,
            11..=50 => top_k * 10,
            51..=100 => top_k * 5,
            _ => top_k * 2,
        };

        let batch_results =
            self.index
                .search_batch_parallel(vectors, overfetch_k, crate::SearchQuality::Balanced);

        let fused = fusion
            .fuse(batch_results)
            .map_err(|e| Error::Config(format!("Fusion error: {e}")))?;

        Ok(fused.into_iter().take(top_k).collect())
    }
}
