//! Similarity scoring and property projection for MATCH queries.
//!
//! Handles `execute_match_with_similarity`, property projection (EPIC-058 US-007),
//! result ordering, and conversion to `SearchResults`.

// Reason: Numeric casts in similarity scoring are intentional:
// - u32->f32 for depth scoring: depth values are small (< 1000)
// - All casts are for internal query execution, not user data validation
#![allow(clippy::cast_precision_loss)]

use super::parse_property_path;
use super::{parse_projection_item, MatchResult, ProjectionItem};
use crate::collection::expiry::{is_payload_expired, now_unix_secs};
use crate::collection::types::Collection;
use crate::error::Result;
use crate::point::SearchResult;
use crate::storage::{LogPayloadStorage, PayloadStorage, VectorStorage};
use crate::validation::validate_dimension_match;
use std::collections::HashMap;

/// Bundled invariant state for projecting one match result's RETURN items.
struct ProjectionCtx<'a> {
    bindings: &'a HashMap<String, u64>,
    edge_bindings: &'a HashMap<String, u64>,
    edge_paths: &'a HashMap<String, Vec<u64>>,
    score: Option<f32>,
    payload_guard: &'a LogPayloadStorage,
}

/// Output key of a RETURN item: its `AS` alias, or the raw expression.
fn projection_key(item: &crate::velesql::ReturnItem) -> String {
    item.alias
        .clone()
        .unwrap_or_else(|| item.expression.clone())
}

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
    /// per-node lock acquisitions during traversal. `edge_bindings` maps
    /// relationship aliases to traversed edge ids so `RETURN r.prop`
    /// projects the EDGE's property (audit 2026-06 F).
    pub(crate) fn project_properties(
        &self,
        bindings: &HashMap<String, u64>,
        edge_bindings: &HashMap<String, u64>,
        edge_paths: &HashMap<String, Vec<u64>>,
        return_clause: &crate::velesql::ReturnClause,
        payload_guard: &LogPayloadStorage,
    ) -> HashMap<String, serde_json::Value> {
        self.project_properties_with_score(
            bindings,
            edge_bindings,
            edge_paths,
            return_clause,
            None,
            payload_guard,
        )
    }

    /// Projects properties with an optional similarity score (Fix #489).
    ///
    /// Uses the pre-acquired `payload_guard` instead of locking per-call.
    /// When `score` is `Some`, `RETURN similarity()` injects it into the
    /// projected map. All other variants work identically to
    /// [`project_properties`].
    pub(crate) fn project_properties_with_score(
        &self,
        bindings: &HashMap<String, u64>,
        edge_bindings: &HashMap<String, u64>,
        edge_paths: &HashMap<String, Vec<u64>>,
        return_clause: &crate::velesql::ReturnClause,
        score: Option<f32>,
        payload_guard: &LogPayloadStorage,
    ) -> HashMap<String, serde_json::Value> {
        let ctx = ProjectionCtx {
            bindings,
            edge_bindings,
            edge_paths,
            score,
            payload_guard,
        };
        let mut projected = HashMap::new();
        for item in &return_clause.items {
            self.project_return_item(&ctx, item, &mut projected);
        }
        projected
    }

    /// Projects one RETURN item into `projected` according to its shape.
    fn project_return_item(
        &self,
        ctx: &ProjectionCtx<'_>,
        item: &crate::velesql::ReturnItem,
        projected: &mut HashMap<String, serde_json::Value>,
    ) {
        match parse_projection_item(&item.expression) {
            ProjectionItem::Wildcard => {
                Self::project_wildcard(ctx.bindings, ctx.payload_guard, projected);
            }
            ProjectionItem::FunctionCall(name) => {
                if let ("similarity", Some(s)) = (name, ctx.score) {
                    projected.insert(
                        "similarity()".to_string(),
                        serde_json::Value::from(f64::from(s)),
                    );
                }
            }
            ProjectionItem::PropertyPath { alias, property } => {
                self.project_aliased_property(ctx, alias, property, item, projected);
            }
            ProjectionItem::BareAlias(alias) => {
                // A variable-length relationship alias binds a LIST of
                // relationships (openCypher): project the edge-id list.
                if let Some(edge_ids) = ctx.edge_paths.get(alias) {
                    projected.insert(projection_key(item), serde_json::json!(edge_ids));
                } else {
                    Self::project_bare_alias(alias, ctx.bindings, ctx.payload_guard, projected);
                }
            }
        }
    }

    /// Projects a dotted property, dispatching on what the alias binds:
    /// a fixed-length edge, a variable-length edge list, or a node.
    fn project_aliased_property(
        &self,
        ctx: &ProjectionCtx<'_>,
        alias: &str,
        property: &str,
        item: &crate::velesql::ReturnItem,
        projected: &mut HashMap<String, serde_json::Value>,
    ) {
        // Relationship aliases project from the traversed edge's
        // properties (audit 2026-06 F).
        if let Some(&edge_id) = ctx.edge_bindings.get(alias) {
            self.project_edge_property(edge_id, property, item, projected);
        } else if let Some(edge_ids) = ctx.edge_paths.get(alias) {
            self.project_edge_path_property(edge_ids, property, item, projected);
        } else {
            Self::project_property_path(
                alias,
                property,
                item,
                ctx.bindings,
                ctx.payload_guard,
                projected,
            );
        }
    }

    /// Projects a single dotted property (e.g., `r.since`) from a bound
    /// relationship alias, reading the traversed edge's properties.
    fn project_edge_property(
        &self,
        edge_id: u64,
        property: &str,
        item: &crate::velesql::ReturnItem,
        projected: &mut HashMap<String, serde_json::Value>,
    ) {
        let Some(edge) = self.edge_store.get_edge(edge_id) else {
            return;
        };
        let Some(value) = super::where_eval::edge_property_path(&edge, property) else {
            return;
        };
        projected.insert(projection_key(item), value.clone());
    }

    /// Projects a dotted property across a variable-length alias's edge list
    /// as a JSON array, positionally aligned with the traversed path (missing
    /// properties yield `null`), mirroring openCypher's `[rel IN r | rel.prop]`.
    fn project_edge_path_property(
        &self,
        edge_ids: &[u64],
        property: &str,
        item: &crate::velesql::ReturnItem,
        projected: &mut HashMap<String, serde_json::Value>,
    ) {
        let values: Vec<serde_json::Value> = edge_ids
            .iter()
            .map(|&edge_id| {
                self.edge_store
                    .get_edge(edge_id)
                    .as_ref()
                    .and_then(|edge| super::where_eval::edge_property_path(edge, property).cloned())
                    .unwrap_or(serde_json::Value::Null)
            })
            .collect();
        projected.insert(projection_key(item), serde_json::Value::Array(values));
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
            projected.insert(projection_key(item), value.clone());
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
                        &result.edge_bindings,
                        &result.edge_paths,
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
    pub(super) fn sort_by_score(results: &mut [MatchResult], higher_is_better: bool) {
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

    /// Applies sort direction to a comparison (reverses when `descending`).
    ///
    /// Shared with the structured ORDER BY evaluator in `order_by.rs`.
    #[inline]
    pub(super) fn apply_direction(cmp: std::cmp::Ordering, descending: bool) -> std::cmp::Ordering {
        if descending {
            cmp.reverse()
        } else {
            cmp
        }
    }

    /// ORDER BY a property path (e.g. `n.name`).
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::Error::GraphNotSupported`] (VELES-018) when the
    /// expression is not a valid `alias.property` path, so an unsupported
    /// clause is reported instead of leaving the results unordered.
    pub(super) fn order_match_results_by_property(
        &self,
        results: &mut [MatchResult],
        order_by: &str,
        descending: bool,
    ) -> Result<()> {
        let Some((alias, property)) = parse_property_path(order_by) else {
            return Err(crate::error::Error::GraphNotSupported(format!(
                "MATCH ORDER BY expression '{order_by}' is not supported \
                 (use similarity(), depth, or alias.property)"
            )));
        };
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
            let cmp = super::super::compare_json_values(a_value.as_ref(), b_value.as_ref());
            Self::apply_direction(cmp, descending)
        });
        Ok(())
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

        let now_secs = now_unix_secs();

        for mr in match_results {
            let base = payload_storage.retrieve(mr.node_id).ok().flatten();
            if is_payload_expired(base.as_ref(), now_secs) {
                continue;
            }
            let vector = vector_storage
                .retrieve(mr.node_id)?
                .unwrap_or_else(Vec::new);
            let payload = Some(build_match_payload(base, &mr));

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

fn build_match_payload(
    base_payload: Option<serde_json::Value>,
    result: &MatchResult,
) -> serde_json::Value {
    let mut object = match base_payload {
        Some(serde_json::Value::Object(map)) => map,
        Some(value) => {
            let mut map = serde_json::Map::new();
            map.insert("_payload".to_string(), value);
            map
        }
        None => serde_json::Map::new(),
    };

    for (key, value) in &result.projected {
        object.insert(key.clone(), value.clone());
    }
    object.insert("_bindings".to_string(), serde_json::json!(result.bindings));
    // Edge identities make parallel-edge rows (same node bindings, different
    // edge) and variable-length paths distinguishable by every consumer.
    if !result.edge_bindings.is_empty() {
        object.insert(
            "_edge_bindings".to_string(),
            serde_json::json!(result.edge_bindings),
        );
    }
    if !result.edge_paths.is_empty() {
        object.insert(
            "_edge_paths".to_string(),
            serde_json::json!(result.edge_paths),
        );
    }
    serde_json::Value::Object(object)
}
