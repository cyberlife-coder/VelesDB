//! Similarity scoring and property projection for MATCH queries.
//!
//! Handles `execute_match_with_similarity`, property projection (EPIC-058 US-007),
//! result ordering, and conversion to `SearchResults`.

// SAFETY: Numeric casts in similarity scoring are intentional:
// - u32->f32 for depth scoring: depth values are small (< 1000)
// - All casts are for internal query execution, not user data validation
#![allow(clippy::cast_precision_loss)]

use super::parse_property_path;
use super::MatchResult;
use crate::collection::types::Collection;
use crate::error::{Error, Result};
use crate::point::SearchResult;
use crate::storage::{PayloadStorage, VectorStorage};
use std::collections::HashMap;

impl Collection {
    /// Projects properties from RETURN clause for a match result (EPIC-058 US-007).
    ///
    /// Resolves property paths like "author.name" by:
    /// 1. Looking up the alias in bindings to get `node_id`
    /// 2. Fetching the payload for that node
    /// 3. Extracting the property value
    pub(crate) fn project_properties(
        &self,
        bindings: &HashMap<String, u64>,
        return_clause: &crate::velesql::ReturnClause,
    ) -> HashMap<String, serde_json::Value> {
        let payload_storage = self.payload_storage.read();
        let mut projected = HashMap::new();

        for item in &return_clause.items {
            // Parse property path (e.g., "author.name" -> ("author", "name"))
            if let Some((alias, property)) = parse_property_path(&item.expression) {
                // Get node_id for this alias
                if let Some(&node_id) = bindings.get(alias) {
                    // Get payload for this node
                    if let Ok(Some(payload)) = payload_storage.retrieve(node_id) {
                        // Extract property value (support nested paths)
                        if let Some(payload_map) = payload.as_object() {
                            if let Some(value) = Self::get_nested_property(payload_map, property) {
                                let key = item
                                    .alias
                                    .clone()
                                    .unwrap_or_else(|| item.expression.clone());
                                projected.insert(key, value.clone());
                            }
                        }
                    }
                }
            }
        }

        projected
    }

    /// Gets a nested property from a JSON object (EPIC-058 US-007).
    ///
    /// Supports paths like "metadata.category" for nested access.
    /// Limited to 10 levels of nesting to prevent abuse.
    pub(crate) fn get_nested_property<'a>(
        payload: &'a serde_json::Map<String, serde_json::Value>,
        path: &str,
    ) -> Option<&'a serde_json::Value> {
        // Limit nesting depth to prevent potential abuse
        const MAX_NESTING_DEPTH: usize = 10;

        let parts: Vec<&str> = path.split('.').collect();

        // Bounds check on nesting depth
        if parts.len() > MAX_NESTING_DEPTH {
            tracing::warn!(
                "Property path '{}' exceeds max nesting depth of {}",
                path,
                MAX_NESTING_DEPTH
            );
            return None;
        }

        let first_key = *parts.first()?;
        let mut current: &serde_json::Value = payload.get(first_key)?;

        for part in parts.iter().skip(1) {
            current = current.as_object()?.get(*part)?;
        }

        Some(current)
    }

    /// Executes a MATCH query with similarity scoring (EPIC-045 US-003).
    ///
    /// This method combines graph pattern matching with vector similarity,
    /// enabling hybrid queries like:
    /// `MATCH (n:Article)-[:CITED]->(m) WHERE similarity(m.embedding, $query) > 0.8 RETURN m`
    ///
    /// # Arguments
    ///
    /// * `match_clause` - The parsed MATCH clause
    /// * `query_vector` - The query vector for similarity scoring
    /// * `similarity_threshold` - Minimum similarity score (0.0 to 1.0)
    /// * `params` - Query parameters
    ///
    /// # Returns
    ///
    /// Vector of `MatchResult` with similarity scores and projected properties.
    ///
    /// # Errors
    ///
    /// Returns an error on dimension mismatch, underlying storage/search errors,
    /// or `ORDER BY` failures.
    pub fn execute_match_with_similarity(
        &self,
        match_clause: &crate::velesql::MatchClause,
        query_vector: &[f32],
        similarity_threshold: f32,
        params: &HashMap<String, serde_json::Value>,
    ) -> Result<Vec<MatchResult>> {
        // First, execute the basic MATCH query
        let results = self.execute_match(match_clause, params)?;

        if results.is_empty() {
            return Ok(results);
        }

        // Get the metric from config
        let config = self.config.read();
        let metric = config.metric;
        let expected_dimension = config.dimension;
        drop(config);

        if query_vector.len() != expected_dimension {
            return Err(Error::DimensionMismatch {
                expected: expected_dimension,
                actual: query_vector.len(),
            });
        }

        // Score each result by similarity/distance
        let vector_storage = self.vector_storage.read();
        let mut scored_results = Vec::new();
        let higher_is_better = metric.higher_is_better();

        for mut result in results {
            // Get vector for this node
            if let Ok(Some(node_vector)) = vector_storage.retrieve(result.node_id) {
                // Storage invariant: vectors must match collection dimension.
                if node_vector.len() != expected_dimension {
                    return Err(Error::DimensionMismatch {
                        expected: expected_dimension,
                        actual: node_vector.len(),
                    });
                }

                // Calculate similarity/distance
                let score = metric.calculate(&node_vector, query_vector);

                // Filter by threshold - metric-aware comparison
                // For similarity metrics (Cosine, DotProduct, Jaccard): higher >= threshold
                // For distance metrics (Euclidean, Hamming): lower <= threshold
                let passes_threshold = if higher_is_better {
                    score >= similarity_threshold
                } else {
                    score <= similarity_threshold
                };

                if passes_threshold {
                    result.score = Some(score);

                    // Project properties from RETURN clause (EPIC-058 US-007)
                    result.projected =
                        self.project_properties(&result.bindings, &match_clause.return_clause);

                    scored_results.push(result);
                }
            }
        }

        // Sort by score - metric-aware ordering
        // For similarity: descending (higher = more similar)
        // For distance: ascending (lower = more similar)
        if higher_is_better {
            scored_results
                .sort_by(|a, b| b.score.unwrap_or(0.0).total_cmp(&a.score.unwrap_or(0.0)));
        } else {
            scored_results.sort_by(|a, b| {
                a.score
                    .unwrap_or(f32::MAX)
                    .total_cmp(&b.score.unwrap_or(f32::MAX))
            });
        }

        Ok(scored_results)
    }

    /// Applies ORDER BY to match results (EPIC-045 US-005).
    ///
    /// Supports ordering by:
    /// - `similarity()` - Vector similarity score
    /// - Property path (e.g., `n.name`)
    /// - Depth
    pub fn order_match_results(results: &mut [MatchResult], order_by: &str, descending: bool) {
        match order_by {
            "similarity()" | "similarity" => {
                results.sort_by(|a, b| {
                    let cmp = a.score.unwrap_or(0.0).total_cmp(&b.score.unwrap_or(0.0));
                    if descending {
                        cmp.reverse()
                    } else {
                        cmp
                    }
                });
            }
            "depth" => {
                results.sort_by(|a, b| {
                    let cmp = a.depth.cmp(&b.depth);
                    if descending {
                        cmp.reverse()
                    } else {
                        cmp
                    }
                });
            }
            _ => {
                // For property paths, we need payload access
                // This is a TODO for future enhancement
            }
        }
    }

    /// Converts `MatchResults` to `SearchResults` for unified API (EPIC-045 US-002).
    ///
    /// This allows MATCH queries to return the same result type as SELECT queries,
    /// enabling consistent downstream processing.
    ///
    /// # Errors
    ///
    /// Returns an error when vector storage access fails for any matched node.
    pub fn match_results_to_search_results(
        &self,
        match_results: Vec<MatchResult>,
    ) -> Result<Vec<SearchResult>> {
        let payload_storage = self.payload_storage.read();
        let vector_storage = self.vector_storage.read();

        let mut results = Vec::new();

        for mr in match_results {
            // Get vector and payload for the node
            let vector = vector_storage
                .retrieve(mr.node_id)?
                .unwrap_or_else(Vec::new);
            let payload = payload_storage.retrieve(mr.node_id).ok().flatten();

            let point = crate::Point {
                id: mr.node_id,
                vector,
                payload,
            };

            // Use depth as inverse score (closer = higher score)
            let score = mr.score.unwrap_or(1.0 / (mr.depth as f32 + 1.0));

            results.push(SearchResult::new(point, score));
        }

        Ok(results)
    }
}
