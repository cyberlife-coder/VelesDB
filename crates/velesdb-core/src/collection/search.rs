//! Search implementation for Collection.

use super::types::Collection;
use crate::error::{Error, Result};
use crate::index::VectorIndex;
use crate::point::{Point, SearchResult};
use crate::storage::{PayloadStorage, VectorStorage};

/// Wrapper for f32 to implement Ord for `BinaryHeap` in hybrid search.
#[derive(Debug, Clone, Copy, PartialEq)]
struct OrderedFloat(f32);

impl Eq for OrderedFloat {}

impl PartialOrd for OrderedFloat {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for OrderedFloat {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0
            .partial_cmp(&other.0)
            .unwrap_or(std::cmp::Ordering::Equal)
    }
}

impl Collection {
    /// Searches for the k nearest neighbors of the query vector.
    ///
    /// Uses HNSW index for fast approximate nearest neighbor search.
    ///
    /// # Errors
    ///
    /// Returns an error if the query vector dimension doesn't match the collection.
    pub fn search(&self, query: &[f32], k: usize) -> Result<Vec<SearchResult>> {
        let config = self.config.read();

        if query.len() != config.dimension {
            return Err(Error::DimensionMismatch {
                expected: config.dimension,
                actual: query.len(),
            });
        }
        drop(config);

        // Use HNSW index for fast ANN search
        let index_results = self.index.search(query, k);

        let vector_storage = self.vector_storage.read();
        let payload_storage = self.payload_storage.read();

        // Map index results to SearchResult with full point data
        let results: Vec<SearchResult> = index_results
            .into_iter()
            .filter_map(|(id, score)| {
                // We need to fetch vector and payload
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

    /// Performs vector similarity search with custom `ef_search` parameter.
    ///
    /// Higher `ef_search` = better recall, slower search.
    /// Default `ef_search` is 128 (Balanced mode).
    ///
    /// # Errors
    ///
    /// Returns an error if the query vector dimension doesn't match the collection.
    pub fn search_with_ef(
        &self,
        query: &[f32],
        k: usize,
        ef_search: usize,
    ) -> Result<Vec<SearchResult>> {
        let config = self.config.read();

        if query.len() != config.dimension {
            return Err(Error::DimensionMismatch {
                expected: config.dimension,
                actual: query.len(),
            });
        }
        drop(config);

        // Convert ef_search to SearchQuality
        let quality = match ef_search {
            0..=64 => crate::SearchQuality::Fast,
            65..=128 => crate::SearchQuality::Balanced,
            129..=256 => crate::SearchQuality::Accurate,
            _ => crate::SearchQuality::Perfect,
        };

        let index_results = self.index.search_with_quality(query, k, quality);

        let vector_storage = self.vector_storage.read();
        let payload_storage = self.payload_storage.read();

        let results: Vec<SearchResult> = index_results
            .into_iter()
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

    /// Performs fast vector similarity search returning only IDs and scores.
    ///
    /// Perf: This is ~3-5x faster than `search()` because it skips vector/payload retrieval.
    /// Use this when you only need IDs and scores, not full point data.
    ///
    /// # Arguments
    ///
    /// * `query` - Query vector
    /// * `k` - Maximum number of results to return
    ///
    /// # Returns
    ///
    /// Vector of (id, score) tuples sorted by similarity.
    ///
    /// # Errors
    ///
    /// Returns an error if the query vector dimension doesn't match the collection.
    pub fn search_ids(&self, query: &[f32], k: usize) -> Result<Vec<(u64, f32)>> {
        let config = self.config.read();

        if query.len() != config.dimension {
            return Err(Error::DimensionMismatch {
                expected: config.dimension,
                actual: query.len(),
            });
        }
        drop(config);

        // Perf: Direct HNSW search without vector/payload retrieval
        let results = self.index.search(query, k);
        Ok(results)
    }

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
        use crate::index::SearchQuality;

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
        use crate::index::SearchQuality;

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

    /// Executes a `VelesQL` query on this collection.
    ///
    /// This method unifies vector search, text search, and metadata filtering
    /// into a single interface.
    ///
    /// # Arguments
    ///
    /// * `query` - Parsed `VelesQL` query
    /// * `params` - Query parameters for resolving placeholders (e.g., $v)
    ///
    /// # Errors
    ///
    /// Returns an error if the query cannot be executed (e.g., missing parameters).
    pub fn execute_query(
        &self,
        query: &crate::velesql::Query,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<Vec<SearchResult>> {
        let stmt = &query.select;
        let limit = usize::try_from(stmt.limit.unwrap_or(10)).unwrap_or(usize::MAX);

        // 1. Extract vector search (NEAR) if present
        let mut vector_search = None;
        let mut filter_condition = None;

        if let Some(ref cond) = stmt.where_clause {
            let mut extracted_cond = cond.clone();
            vector_search = self.extract_vector_search(&mut extracted_cond, params)?;
            filter_condition = Some(extracted_cond);
        }

        // 2. Resolve WITH clause options
        let mut ef_search = None;
        if let Some(ref with) = stmt.with_clause {
            ef_search = with.get_ef_search();
        }

        // 3. Execute query based on extracted components
        let results = match (vector_search, filter_condition) {
            (Some(vector), Some(ref cond)) => {
                // Check if condition contains MATCH for hybrid search
                if let Some(text_query) = Self::extract_match_query(cond) {
                    // Hybrid search: NEAR + MATCH
                    self.hybrid_search(&vector, &text_query, limit, None)?
                } else {
                    // Vector search with metadata filter
                    let filter =
                        crate::filter::Filter::new(crate::filter::Condition::from(cond.clone()));
                    self.search_with_filter(&vector, limit, &filter)?
                }
            }
            (Some(vector), None) => {
                // Pure vector search
                if let Some(ef) = ef_search {
                    self.search_with_ef(&vector, limit, ef)?
                } else {
                    self.search(&vector, limit)?
                }
            }
            (None, Some(cond)) => {
                // Metadata-only filter (table scan + filter)
                // If it's a MATCH condition, use text search
                if let crate::velesql::Condition::Match(ref m) = cond {
                    // Pure text search - no filter needed
                    self.text_search(&m.query, limit)
                } else {
                    // Generic metadata filter: perform a scan (fallback)
                    let filter = crate::filter::Filter::new(crate::filter::Condition::from(cond));
                    self.execute_scan_query(&filter, limit)
                }
            }
            (None, None) => {
                // SELECT * FROM docs LIMIT N (no WHERE)
                self.execute_scan_query(
                    &crate::filter::Filter::new(crate::filter::Condition::And {
                        conditions: vec![],
                    }),
                    limit,
                )
            }
        };

        Ok(results)
    }

    /// Helper to extract MATCH query from any nested condition.
    fn extract_match_query(condition: &crate::velesql::Condition) -> Option<String> {
        use crate::velesql::Condition;
        match condition {
            Condition::Match(m) => Some(m.query.clone()),
            Condition::And(left, right) => {
                Self::extract_match_query(left).or_else(|| Self::extract_match_query(right))
            }
            Condition::Group(inner) => Self::extract_match_query(inner),
            _ => None,
        }
    }

    /// Internal helper to extract vector search from WHERE clause.
    #[allow(clippy::self_only_used_in_recursion)]
    fn extract_vector_search(
        &self,
        condition: &mut crate::velesql::Condition,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<Option<Vec<f32>>> {
        use crate::velesql::{Condition, VectorExpr};

        match condition {
            Condition::VectorSearch(vs) => {
                let vec = match &vs.vector {
                    VectorExpr::Literal(v) => v.clone(),
                    VectorExpr::Parameter(name) => {
                        let val = params.get(name).ok_or_else(|| {
                            Error::Config(format!("Missing query parameter: ${name}"))
                        })?;
                        if let serde_json::Value::Array(arr) = val {
                            #[allow(clippy::cast_possible_truncation)]
                            arr.iter()
                                .map(|v| {
                                    v.as_f64().map(|f| f as f32).ok_or_else(|| {
                                        Error::Config(format!(
                                            "Invalid vector parameter ${name}: expected numbers"
                                        ))
                                    })
                                })
                                .collect::<Result<Vec<f32>>>()?
                        } else {
                            return Err(Error::Config(format!(
                                "Invalid vector parameter ${name}: expected array"
                            )));
                        }
                    }
                };
                Ok(Some(vec))
            }
            Condition::And(left, right) => {
                if let Some(v) = self.extract_vector_search(left, params)? {
                    return Ok(Some(v));
                }
                self.extract_vector_search(right, params)
            }
            Condition::Group(inner) => self.extract_vector_search(inner, params),
            _ => Ok(None),
        }
    }

    /// Fallback method for metadata-only queries without vector search.
    fn execute_scan_query(
        &self,
        filter: &crate::filter::Filter,
        limit: usize,
    ) -> Vec<SearchResult> {
        let payload_storage = self.payload_storage.read();
        let vector_storage = self.vector_storage.read();

        // Scan all points (slow fallback)
        // In production, this should use metadata indexes
        let mut results = Vec::new();

        // We need all IDs to scan
        let ids = vector_storage.ids();

        for id in ids {
            let payload = payload_storage.retrieve(id).ok().flatten();
            let matches = match payload {
                Some(ref p) => filter.matches(p),
                None => filter.matches(&serde_json::Value::Null),
            };

            if matches {
                if let Ok(Some(vector)) = vector_storage.retrieve(id) {
                    results.push(SearchResult::new(
                        Point {
                            id,
                            vector,
                            payload,
                        },
                        1.0, // Constant score for scans
                    ));
                }
            }

            if results.len() >= limit {
                break;
            }
        }

        results
    }

    /// Performs full-text search using BM25.
    ///
    /// # Arguments
    ///
    /// * `query` - Text query to search for
    /// * `k` - Maximum number of results to return
    ///
    /// # Returns
    ///
    /// Vector of search results sorted by BM25 score (descending).
    #[must_use]
    pub fn text_search(&self, query: &str, k: usize) -> Vec<SearchResult> {
        let bm25_results = self.text_index.search(query, k);

        let vector_storage = self.vector_storage.read();
        let payload_storage = self.payload_storage.read();

        bm25_results
            .into_iter()
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
            .collect()
    }

    /// Performs hybrid search combining vector similarity and full-text search.
    ///
    /// Uses Reciprocal Rank Fusion (RRF) to combine results from both searches.
    ///
    /// # Arguments
    ///
    /// * `vector_query` - Query vector for similarity search
    /// * `text_query` - Text query for BM25 search
    /// * `k` - Maximum number of results to return
    /// * `vector_weight` - Weight for vector results (0.0-1.0, default 0.5)
    ///
    /// # Performance (v0.9+)
    ///
    /// - **Streaming RRF**: `BinaryHeap` maintains top-k during fusion (O(n log k) vs O(n log n))
    /// - **Vector-first gating**: Text search limited to 2k candidates for efficiency
    /// - **`FxHashMap`**: Faster hashing for score aggregation
    ///
    /// # Errors
    ///
    /// Returns an error if the query vector dimension doesn't match.
    pub fn hybrid_search(
        &self,
        vector_query: &[f32],
        text_query: &str,
        k: usize,
        vector_weight: Option<f32>,
    ) -> Result<Vec<SearchResult>> {
        use std::cmp::Reverse;
        use std::collections::BinaryHeap;

        let config = self.config.read();
        if vector_query.len() != config.dimension {
            return Err(Error::DimensionMismatch {
                expected: config.dimension,
                actual: vector_query.len(),
            });
        }
        drop(config);

        let weight = vector_weight.unwrap_or(0.5).clamp(0.0, 1.0);
        let text_weight = 1.0 - weight;

        // Get vector search results (more than k to allow for fusion)
        let vector_results = self.index.search(vector_query, k * 2);

        // Get BM25 text search results
        let text_results = self.text_index.search(text_query, k * 2);

        // Perf: Apply RRF with FxHashMap for faster hashing
        // RRF score = weight / (rank + 60) - the constant 60 is standard (Cormack et al.)
        let mut fused_scores: rustc_hash::FxHashMap<u64, f32> =
            rustc_hash::FxHashMap::with_capacity_and_hasher(
                vector_results.len() + text_results.len(),
                rustc_hash::FxBuildHasher,
            );

        // Add vector scores with RRF
        #[allow(clippy::cast_precision_loss)]
        for (rank, (id, _)) in vector_results.iter().enumerate() {
            let rrf_score = weight / (rank as f32 + 60.0);
            *fused_scores.entry(*id).or_insert(0.0) += rrf_score;
        }

        // Add text scores with RRF
        #[allow(clippy::cast_precision_loss)]
        for (rank, (id, _)) in text_results.iter().enumerate() {
            let rrf_score = text_weight / (rank as f32 + 60.0);
            *fused_scores.entry(*id).or_insert(0.0) += rrf_score;
        }

        // Perf: Streaming top-k with BinaryHeap (O(n log k) vs O(n log n) for full sort)
        // Use min-heap of size k: always keep the k highest scores
        let mut top_k: BinaryHeap<Reverse<(OrderedFloat, u64)>> = BinaryHeap::with_capacity(k + 1);

        for (id, score) in fused_scores {
            top_k.push(Reverse((OrderedFloat(score), id)));
            if top_k.len() > k {
                top_k.pop(); // Remove smallest
            }
        }

        // Extract and sort descending
        let mut scored_ids: Vec<(u64, f32)> = top_k
            .into_iter()
            .map(|Reverse((OrderedFloat(s), id))| (id, s))
            .collect();
        scored_ids.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        // Fetch full point data
        let vector_storage = self.vector_storage.read();
        let payload_storage = self.payload_storage.read();

        let results: Vec<SearchResult> = scored_ids
            .into_iter()
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

    /// Searches for the k nearest neighbors with metadata filtering.
    ///
    /// Performs post-filtering: retrieves more candidates from HNSW,
    /// then filters by metadata conditions.
    ///
    /// # Arguments
    ///
    /// * `query` - Query vector
    /// * `k` - Maximum number of results to return
    /// * `filter` - Metadata filter to apply
    ///
    /// # Errors
    ///
    /// Returns an error if the query vector dimension doesn't match the collection.
    pub fn search_with_filter(
        &self,
        query: &[f32],
        k: usize,
        filter: &crate::filter::Filter,
    ) -> Result<Vec<SearchResult>> {
        let config = self.config.read();

        if query.len() != config.dimension {
            return Err(Error::DimensionMismatch {
                expected: config.dimension,
                actual: query.len(),
            });
        }
        drop(config);

        // Post-filtering strategy: retrieve more candidates than k, then filter
        // Heuristic: retrieve 4x candidates to account for filtered-out results
        let candidates_k = k.saturating_mul(4).max(k + 10);
        let index_results = self.index.search(query, candidates_k);

        let vector_storage = self.vector_storage.read();
        let payload_storage = self.payload_storage.read();

        // Map index results to SearchResult with full point data, applying filter
        let mut results: Vec<SearchResult> = index_results
            .into_iter()
            .filter_map(|(id, score)| {
                let vector = vector_storage.retrieve(id).ok().flatten()?;
                let payload = payload_storage.retrieve(id).ok().flatten();

                // Apply filter - if no payload, filter fails
                let payload_ref = payload.as_ref()?;
                if !filter.matches(payload_ref) {
                    return None;
                }

                let point = Point {
                    id,
                    vector,
                    payload,
                };

                Some(SearchResult::new(point, score))
            })
            .take(k)
            .collect();

        // Ensure results are sorted by score (should already be, but defensive)
        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        Ok(results)
    }

    /// Performs full-text search with metadata filtering.
    ///
    /// # Arguments
    ///
    /// * `query` - Text query to search for
    /// * `k` - Maximum number of results to return
    /// * `filter` - Metadata filter to apply
    ///
    /// # Returns
    ///
    /// Vector of search results sorted by BM25 score (descending).
    #[must_use]
    pub fn text_search_with_filter(
        &self,
        query: &str,
        k: usize,
        filter: &crate::filter::Filter,
    ) -> Vec<SearchResult> {
        // Retrieve more candidates for filtering
        let candidates_k = k.saturating_mul(4).max(k + 10);
        let bm25_results = self.text_index.search(query, candidates_k);

        let vector_storage = self.vector_storage.read();
        let payload_storage = self.payload_storage.read();

        bm25_results
            .into_iter()
            .filter_map(|(id, score)| {
                let vector = vector_storage.retrieve(id).ok().flatten()?;
                let payload = payload_storage.retrieve(id).ok().flatten();

                // Apply filter - if no payload, filter fails
                let payload_ref = payload.as_ref()?;
                if !filter.matches(payload_ref) {
                    return None;
                }

                let point = Point {
                    id,
                    vector,
                    payload,
                };

                Some(SearchResult::new(point, score))
            })
            .take(k)
            .collect()
    }

    /// Performs hybrid search (vector + text) with metadata filtering.
    ///
    /// Uses Reciprocal Rank Fusion (RRF) to combine results from both searches,
    /// then applies metadata filter.
    ///
    /// # Arguments
    ///
    /// * `vector_query` - Query vector for similarity search
    /// * `text_query` - Text query for BM25 search
    /// * `k` - Maximum number of results to return
    /// * `vector_weight` - Weight for vector results (0.0-1.0, default 0.5)
    /// * `filter` - Metadata filter to apply
    ///
    /// # Errors
    ///
    /// Returns an error if the query vector dimension doesn't match.
    pub fn hybrid_search_with_filter(
        &self,
        vector_query: &[f32],
        text_query: &str,
        k: usize,
        vector_weight: Option<f32>,
        filter: &crate::filter::Filter,
    ) -> Result<Vec<SearchResult>> {
        let config = self.config.read();
        if vector_query.len() != config.dimension {
            return Err(Error::DimensionMismatch {
                expected: config.dimension,
                actual: vector_query.len(),
            });
        }
        drop(config);

        let weight = vector_weight.unwrap_or(0.5).clamp(0.0, 1.0);
        let text_weight = 1.0 - weight;

        // Get more candidates for filtering
        let candidates_k = k.saturating_mul(4).max(k + 10);

        // Get vector search results
        let vector_results = self.index.search(vector_query, candidates_k);

        // Get BM25 text search results
        let text_results = self.text_index.search(text_query, candidates_k);

        // Apply RRF (Reciprocal Rank Fusion)
        let mut fused_scores: rustc_hash::FxHashMap<u64, f32> = rustc_hash::FxHashMap::default();

        #[allow(clippy::cast_precision_loss)]
        for (rank, (id, _)) in vector_results.iter().enumerate() {
            let rrf_score = weight / (rank as f32 + 60.0);
            *fused_scores.entry(*id).or_insert(0.0) += rrf_score;
        }

        #[allow(clippy::cast_precision_loss)]
        for (rank, (id, _)) in text_results.iter().enumerate() {
            let rrf_score = text_weight / (rank as f32 + 60.0);
            *fused_scores.entry(*id).or_insert(0.0) += rrf_score;
        }

        // Sort by fused score
        let mut scored_ids: Vec<_> = fused_scores.into_iter().collect();
        scored_ids.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        // Fetch full point data and apply filter
        let vector_storage = self.vector_storage.read();
        let payload_storage = self.payload_storage.read();

        let results: Vec<SearchResult> = scored_ids
            .into_iter()
            .filter_map(|(id, score)| {
                let vector = vector_storage.retrieve(id).ok().flatten()?;
                let payload = payload_storage.retrieve(id).ok().flatten();

                // Apply filter - if no payload, filter fails
                let payload_ref = payload.as_ref()?;
                if !filter.matches(payload_ref) {
                    return None;
                }

                let point = Point {
                    id,
                    vector,
                    payload,
                };

                Some(SearchResult::new(point, score))
            })
            .take(k)
            .collect();

        Ok(results)
    }
}
