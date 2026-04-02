//! Similarity scoring and property projection for MATCH queries.
//!
//! Handles `execute_match_with_similarity`, property projection (EPIC-058 US-007),
//! result ordering, and conversion to `SearchResults`.

// SAFETY: Numeric casts in similarity scoring are intentional:
// - u32->f32 for depth scoring: depth values are small (< 1000)
// - All casts are for internal query execution, not user data validation
#![allow(clippy::cast_precision_loss)]

use super::parse_property_path;
use super::{parse_projection_item, MatchResult, ProjectionItem};
use crate::collection::types::Collection;
use crate::error::Result;
use crate::point::SearchResult;
use crate::storage::{LogPayloadStorage, PayloadStorage, VectorStorage};
use crate::validation::validate_dimension_match;
use std::collections::HashMap;

impl Collection {
    /// Projects properties from RETURN clause for a match result (Fix #489).
    ///
    /// Dispatches each RETURN item to variant-specific projection logic:
    /// - `Wildcard`: all properties from all bound nodes
    /// - `FunctionCall("similarity")`: injects the similarity score if available
    /// - `PropertyPath`: a single dotted property from one bound node
    /// - `BareAlias`: all properties from a single bound node
    ///
    /// The caller must pass a pre-acquired `payload_guard` to avoid
    /// per-node lock acquisitions during traversal.
    #[allow(clippy::unused_self)] // Method on Collection for API consistency
    pub(crate) fn project_properties(
        &self,
        bindings: &HashMap<String, u64>,
        return_clause: &crate::velesql::ReturnClause,
        payload_guard: &LogPayloadStorage,
    ) -> HashMap<String, serde_json::Value> {
        self.project_properties_with_score(bindings, return_clause, None, payload_guard)
    }

    /// Projects properties with an optional similarity score (Fix #489).
    ///
    /// Uses the pre-acquired `payload_guard` instead of locking per-call.
    /// When `score` is `Some`, `RETURN similarity()` injects it into the
    /// projected map. All other variants work identically to
    /// [`project_properties`].
    #[allow(clippy::unused_self)] // Method on Collection for API consistency
    pub(crate) fn project_properties_with_score(
        &self,
        bindings: &HashMap<String, u64>,
        return_clause: &crate::velesql::ReturnClause,
        score: Option<f32>,
        payload_guard: &LogPayloadStorage,
    ) -> HashMap<String, serde_json::Value> {
        let mut projected = HashMap::new();

        for item in &return_clause.items {
            match parse_projection_item(&item.expression) {
                ProjectionItem::Wildcard => {
                    Self::project_wildcard(bindings, payload_guard, &mut projected);
                }
                ProjectionItem::FunctionCall(name) => {
                    if name == "similarity" {
                        if let Some(s) = score {
                            projected.insert(
                                "similarity()".to_string(),
                                serde_json::Value::from(f64::from(s)),
                            );
                        }
                    }
                }
                ProjectionItem::PropertyPath { alias, property } => {
                    Self::project_property_path(
                        alias,
                        property,
                        item,
                        bindings,
                        payload_guard,
                        &mut projected,
                    );
                }
                ProjectionItem::BareAlias(alias) => {
                    Self::project_bare_alias(alias, bindings, payload_guard, &mut projected);
                }
            }
        }

        projected
    }

    /// Projects ALL properties from ALL bound nodes into the result (RETURN *).
    fn project_wildcard(
        bindings: &HashMap<String, u64>,
        payload_storage: &crate::storage::LogPayloadStorage,
        projected: &mut HashMap<String, serde_json::Value>,
    ) {
        for (alias, &node_id) in bindings {
            Self::project_all_node_properties(alias, node_id, payload_storage, projected);
        }
    }

    /// Inserts all payload properties of a single node into `projected`,
    /// prefixed with `alias.` (shared by `project_wildcard` and `project_bare_alias`).
    fn project_all_node_properties(
        alias: &str,
        node_id: u64,
        payload_storage: &crate::storage::LogPayloadStorage,
        projected: &mut HashMap<String, serde_json::Value>,
    ) {
        let Ok(Some(payload)) = payload_storage.retrieve(node_id) else {
            return;
        };
        if let Some(map) = payload.as_object() {
            for (key, value) in map {
                projected.insert(format!("{alias}.{key}"), value.clone());
            }
        }
    }

    /// Projects a single dotted property (e.g., `n.name`) from a bound node.
    fn project_property_path(
        alias: &str,
        property: &str,
        item: &crate::velesql::ReturnItem,
        bindings: &HashMap<String, u64>,
        payload_storage: &crate::storage::LogPayloadStorage,
        projected: &mut HashMap<String, serde_json::Value>,
    ) {
        let Some(&node_id) = bindings.get(alias) else {
            return;
        };
        let Ok(Some(payload)) = payload_storage.retrieve(node_id) else {
            return;
        };
        let Some(payload_map) = payload.as_object() else {
            return;
        };
        if let Some(value) = Self::get_nested_property(payload_map, property) {
            let key = item
                .alias
                .clone()
                .unwrap_or_else(|| item.expression.clone());
            projected.insert(key, value.clone());
        }
    }

    /// Projects ALL properties from a single bound node (RETURN n).
    fn project_bare_alias(
        alias: &str,
        bindings: &HashMap<String, u64>,
        payload_storage: &crate::storage::LogPayloadStorage,
        projected: &mut HashMap<String, serde_json::Value>,
    ) {
        let Some(&node_id) = bindings.get(alias) else {
            return;
        };
        Self::project_all_node_properties(alias, node_id, payload_storage, projected);
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
    /// Combines graph pattern matching with vector similarity, enabling queries
    /// like `MATCH (n:Article)-[:CITED]->(m) WHERE similarity(...) > 0.8`.
    ///
    /// Acquires `payload_storage` and `vector_storage` once for the entire
    /// scoring loop to avoid per-node lock acquisitions.
    ///
    /// # Errors
    ///
    /// Returns an error on dimension mismatch or underlying storage errors.
    #[allow(clippy::too_many_lines)]
    pub fn execute_match_with_similarity(
        &self,
        match_clause: &crate::velesql::MatchClause,
        query_vector: &[f32],
        similarity_threshold: f32,
        params: &HashMap<String, serde_json::Value>,
    ) -> Result<Vec<MatchResult>> {
        let results = self.execute_match(match_clause, params)?;

        if results.is_empty() {
            return Ok(results);
        }

        let config = self.config.read();
        let metric = config.metric;
        let expected_dimension = config.dimension;
        drop(config);

        validate_dimension_match(expected_dimension, query_vector.len())?;

        // Hoist both storage locks once for the entire scoring loop.
        let payload_guard = self.payload_storage.read();
        let vector_storage = self.vector_storage.read();
        let higher_is_better = metric.higher_is_better();

        let mut scored_results = self.score_match_results(
            results,
            &vector_storage,
            &payload_guard,
            match_clause,
            query_vector,
            expected_dimension,
            metric,
            similarity_threshold,
            higher_is_better,
        )?;

        Self::sort_by_score(&mut scored_results, higher_is_better);

        Ok(scored_results)
    }

    /// Scores each match result by vector similarity against the query vector.
    ///
    /// Filters results below the threshold and projects RETURN properties.
    #[allow(clippy::too_many_arguments)]
    fn score_match_results(
        &self,
        results: Vec<MatchResult>,
        vector_storage: &crate::storage::MmapStorage,
        payload_guard: &LogPayloadStorage,
        match_clause: &crate::velesql::MatchClause,
        query_vector: &[f32],
        expected_dimension: usize,
        metric: crate::distance::DistanceMetric,
        similarity_threshold: f32,
        higher_is_better: bool,
    ) -> Result<Vec<MatchResult>> {
        let mut scored_results = Vec::new();

        for mut result in results {
            if let Ok(Some(node_vector)) = vector_storage.retrieve(result.node_id) {
                validate_dimension_match(expected_dimension, node_vector.len())?;

                let score = metric.calculate(&node_vector, query_vector);

                let passes_threshold = if higher_is_better {
                    score >= similarity_threshold
                } else {
                    score <= similarity_threshold
                };

                if passes_threshold {
                    result.score = Some(score);
                    result.projected = self.project_properties_with_score(
                        &result.bindings,
                        &match_clause.return_clause,
                        Some(score),
                        payload_guard,
                    );
                    scored_results.push(result);
                }
            }
        }

        Ok(scored_results)
    }

    /// Sorts scored results by similarity — descending for similarity metrics,
    /// ascending for distance metrics.
    fn sort_by_score(results: &mut [MatchResult], higher_is_better: bool) {
        if higher_is_better {
            results.sort_by(|a, b| b.score.unwrap_or(0.0).total_cmp(&a.score.unwrap_or(0.0)));
        } else {
            results.sort_by(|a, b| {
                a.score
                    .unwrap_or(f32::MAX)
                    .total_cmp(&b.score.unwrap_or(f32::MAX))
            });
        }
    }

    /// Applies ORDER BY to match results (EPIC-045 US-005).
    ///
    /// Supports ordering by:
    /// - `similarity()` - Vector similarity score
    /// - Property path (e.g., `n.name`)
    /// - Depth
    pub fn order_match_results(
        &self,
        results: &mut [MatchResult],
        order_by: &str,
        descending: bool,
    ) {
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
                if let Some((alias, property)) = parse_property_path(order_by) {
                    let payload_storage = self.payload_storage.read();
                    results.sort_by(|a, b| {
                        let get_value = |r: &MatchResult| -> Option<serde_json::Value> {
                            let node_id = *r.bindings.get(alias)?;
                            let payload = payload_storage.retrieve(node_id).ok().flatten()?;
                            let object = payload.as_object()?;
                            Self::get_nested_property(object, property).cloned()
                        };

                        let a_value = get_value(a);
                        let b_value = get_value(b);
                        let cmp =
                            super::super::compare_json_values(a_value.as_ref(), b_value.as_ref());
                        if descending {
                            cmp.reverse()
                        } else {
                            cmp
                        }
                    });
                } else {
                    tracing::warn!("Unsupported MATCH ORDER BY expression '{}'", order_by);
                }
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
                sparse_vectors: None,
            };

            // Use depth as inverse score (closer = higher score)
            let score = mr.score.unwrap_or(1.0 / (mr.depth as f32 + 1.0));

            results.push(SearchResult::new(point, score));
        }

        Ok(results)
    }
}
