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
use crate::api_types::serde_id;
use crate::collection::expiry::{is_payload_expired, now_unix_secs};
use crate::collection::types::Collection;
use crate::distance::rank_scores;
use crate::error::Result;
use crate::point::SearchResult;
use crate::storage::{PayloadStorage, VectorStorage};
use crate::validation::validate_dimension_match;
use std::collections::HashMap;

/// Bundled invariant state for projecting one match result's RETURN items.
struct ProjectionCtx<'a> {
    bindings: &'a HashMap<String, u64>,
    edge_bindings: &'a HashMap<String, u64>,
    edge_paths: &'a HashMap<String, Vec<u64>>,
    score: Option<f32>,
    memo: &'a super::PayloadMemo<'a>,
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
    pub(in crate::collection::search::query) fn project_properties(
        &self,
        bindings: &HashMap<String, u64>,
        edge_bindings: &HashMap<String, u64>,
        edge_paths: &HashMap<String, Vec<u64>>,
        return_clause: &crate::velesql::ReturnClause,
        memo: &super::PayloadMemo<'_>,
    ) -> HashMap<String, serde_json::Value> {
        self.project_properties_with_score(
            bindings,
            edge_bindings,
            edge_paths,
            return_clause,
            None,
            memo,
        )
    }

    /// Projects properties with an optional similarity score (Fix #489).
    ///
    /// Uses the pre-acquired `payload_guard` instead of locking per-call.
    /// When `score` is `Some`, `RETURN similarity()` injects it into the
    /// projected map. All other variants work identically to
    /// [`Self::project_properties`].
    pub(in crate::collection::search::query) fn project_properties_with_score(
        &self,
        bindings: &HashMap<String, u64>,
        edge_bindings: &HashMap<String, u64>,
        edge_paths: &HashMap<String, Vec<u64>>,
        return_clause: &crate::velesql::ReturnClause,
        score: Option<f32>,
        memo: &super::PayloadMemo<'_>,
    ) -> HashMap<String, serde_json::Value> {
        let ctx = ProjectionCtx {
            bindings,
            edge_bindings,
            edge_paths,
            score,
            memo,
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
                Self::project_wildcard(ctx.bindings, ctx.memo, projected);
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
                    projected.insert(projection_key(item), serde_id::ids_to_json(edge_ids));
                } else {
                    Self::project_bare_alias(alias, ctx.bindings, ctx.memo, projected);
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
            Self::project_property_path(alias, property, item, ctx.bindings, ctx.memo, projected);
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
        let Some(edge) = self.graph.edge_store.get_edge(edge_id) else {
            return;
        };
        let Some(value) = projected_edge_property(edge_id, &edge, property) else {
            return;
        };
        projected.insert(projection_key(item), value);
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
                self.graph
                    .edge_store
                    .get_edge(edge_id)
                    .as_ref()
                    .and_then(|edge| projected_edge_property(edge_id, edge, property))
                    .unwrap_or(serde_json::Value::Null)
            })
            .collect();
        projected.insert(projection_key(item), serde_json::Value::Array(values));
    }

    /// Projects ALL properties from ALL bound nodes into the result (RETURN *).
    fn project_wildcard(
        bindings: &HashMap<String, u64>,
        memo: &super::PayloadMemo<'_>,
        projected: &mut HashMap<String, serde_json::Value>,
    ) {
        for (alias, &node_id) in bindings {
            Self::project_all_node_properties(alias, node_id, memo, projected);
        }
    }

    /// Inserts all payload properties of a single node into `projected`,
    /// prefixed with `alias.` (shared by `project_wildcard` and `project_bare_alias`),
    /// and the node id under `alias.id`, as everywhere in a MATCH, whatever
    /// the payload holds.
    fn project_all_node_properties(
        alias: &str,
        node_id: u64,
        memo: &super::PayloadMemo<'_>,
        projected: &mut HashMap<String, serde_json::Value>,
    ) {
        let payload = memo.get_bound(Some(alias), node_id);
        if let Some(map) = payload.as_deref().and_then(|p| p.as_object()) {
            for (key, value) in map.iter().filter(|(key, _)| key.as_str() != "id") {
                projected.insert(format!("{alias}.{key}"), value.clone());
            }
        }
        projected.insert(format!("{alias}.id"), serde_json::Value::from(node_id));
    }

    /// Projects a single dotted property (e.g., `n.name`) from a bound node.
    fn project_property_path(
        alias: &str,
        property: &str,
        item: &crate::velesql::ReturnItem,
        bindings: &HashMap<String, u64>,
        memo: &super::PayloadMemo<'_>,
        projected: &mut HashMap<String, serde_json::Value>,
    ) {
        let Some(&node_id) = bindings.get(alias) else {
            return;
        };
        let payload = memo.get_bound(Some(alias), node_id);
        let value = super::where_eval::element_property(node_id, property, |path| {
            Self::get_nested_property(payload.as_deref()?.as_object()?, path)
        });
        if let Some(value) = value {
            projected.insert(projection_key(item), value.into_owned());
        }
    }

    /// Projects ALL properties from a single bound node (RETURN n).
    fn project_bare_alias(
        alias: &str,
        bindings: &HashMap<String, u64>,
        memo: &super::PayloadMemo<'_>,
        projected: &mut HashMap<String, serde_json::Value>,
    ) {
        let Some(&node_id) = bindings.get(alias) else {
            return;
        };
        Self::project_all_node_properties(alias, node_id, memo, projected);
    }

    /// A node payload's property at dotted `path` (`metadata.category`), read
    /// as every clause reads it ([`crate::velesql::json_path::payload_path`]).
    pub(crate) fn get_nested_property<'a>(
        payload: &'a serde_json::Map<String, serde_json::Value>,
        path: &str,
    ) -> Option<&'a serde_json::Value> {
        let (key, rest) = path
            .split_once('.')
            .map_or((path, None), |(k, r)| (k, Some(r)));
        let value = payload.get(key)?;
        rest.map_or(Some(value), |rest| {
            crate::velesql::json_path::payload_path(value, rest)
        })
    }

    /// Executes a MATCH query with similarity scoring (EPIC-045 US-003).
    ///
    /// Combines graph pattern matching with vector similarity, enabling queries
    /// like `MATCH (n:Article)-[:CITED]->(m) WHERE similarity(...) > 0.8`.
    ///
    /// Each row's last node is scored against `query_vector`. A row is kept
    /// when its score is at least `similarity_threshold` under a similarity
    /// metric, at most it under a distance metric
    /// ([`DistanceMetric::higher_is_better`](crate::DistanceMetric::higher_is_better));
    /// `None` keeps every scored row.
    ///
    /// Acquires `payload_storage` and `vector_storage` once for the entire
    /// scoring loop to avoid per-node lock acquisitions.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Query`](crate::error::Error::Query) for a non-finite
    /// threshold, before any row is read, and an error on dimension mismatch
    /// or underlying storage errors.
    pub fn execute_match_with_similarity(
        &self,
        match_clause: &crate::velesql::MatchClause,
        query_vector: &[f32],
        similarity_threshold: Option<f32>,
        params: &HashMap<String, serde_json::Value>,
    ) -> Result<Vec<MatchResult>> {
        self.execute_match_with_similarity_reading(
            match_clause,
            query_vector,
            similarity_threshold,
            params,
            &super::MatchSources::default(),
        )
    }

    /// [`Self::execute_match_with_similarity`] whose `@collection`-annotated
    /// nodes read `sources` too: the database's entry point for REST
    /// `/match` with a query vector.
    ///
    /// # Errors
    ///
    /// As [`Self::execute_match_with_similarity`].
    pub(crate) fn execute_match_with_similarity_reading(
        &self,
        match_clause: &crate::velesql::MatchClause,
        query_vector: &[f32],
        similarity_threshold: Option<f32>,
        params: &HashMap<String, serde_json::Value>,
        sources: &super::MatchSources,
    ) -> Result<Vec<MatchResult>> {
        self.validate_scored_match(match_clause, query_vector, similarity_threshold, params)?;
        // Raw traversal results (no ORDER BY/LIMIT yet): the similarity score is
        // computed below, then the shared finalize step applies ORDER BY + LIMIT.
        // The score sort reorders the rows, so the walk reads every one of them
        // instead of stopping at LIMIT.
        let results = self.execute_match_reading(
            match_clause,
            params,
            None,
            sources,
            super::RowOrder::Rescored,
        )?;

        if results.is_empty() {
            return Ok(results);
        }

        let config = self.storage.config.read();
        let metric = config.metric;
        let expected_dimension = config.dimension;
        drop(config);

        // Hoist every storage lock once for the entire scoring loop, in the
        // cross-collection order (vector_storage(2) before payload_storage(3)
        // within a collection — see tests/tokio_blocking_lock_contention_repro.rs).
        let reads = super::MatchReads::acquire(self, sources);
        let memo = super::PayloadMemo::with_annotations(&reads.payload, reads.annotations());
        let higher_is_better = metric.higher_is_better();

        let mut scored_results = self.score_match_results(
            results,
            &reads.vector,
            &memo,
            match_clause,
            query_vector,
            expected_dimension,
            metric,
            similarity_threshold,
            higher_is_better,
        )?;

        // Release every guard BEFORE ordering: the ORDER BY paths
        // (`order_by.rs`) acquire `config`, `vector_storage` and
        // `payload_storage` themselves, so keeping these guards alive across
        // `apply_match_order_by` would re-acquire locks this thread already
        // holds — a deadlock as soon as one writer queues (parking_lot
        // task-fair semantics) — and take `config` (rank 1) under ranks 2/3.
        drop(memo);
        drop(reads);

        Self::sort_by_score(&mut scored_results, higher_is_better);

        // A RETURN ORDER BY (when present) overrides the implicit score sort;
        // then apply the shared post-sort LIMIT so this vector branch matches
        // the SQL `/query` pipeline instead of returning score-ordered results.
        self.apply_match_order_by(&mut scored_results, match_clause, params, sources)?;
        if let Some(limit) = super::super::match_dispatch::match_return_limit(match_clause) {
            scored_results.truncate(limit);
        }

        Ok(scored_results)
    }

    /// Refuses a scored MATCH before the walk: a threshold core cannot apply,
    /// a clause `QueryValidator` refuses, then the scoring vector and every
    /// `similarity()` query vector the collection cannot score — a walk that
    /// finds no row must not turn a refusal into `[]`.
    fn validate_scored_match(
        &self,
        match_clause: &crate::velesql::MatchClause,
        query_vector: &[f32],
        similarity_threshold: Option<f32>,
        params: &HashMap<String, serde_json::Value>,
    ) -> Result<()> {
        crate::velesql::validate_match_threshold(Some(query_vector), similarity_threshold)?;
        super::super::match_dispatch::validate_match_clause(match_clause)?;
        self.check_match_query_vectors(match_clause, params, Some(query_vector))
    }

    /// Scores each match result by vector similarity against the query vector.
    ///
    /// Filters results below the threshold and projects RETURN properties.
    #[allow(clippy::too_many_arguments)]
    fn score_match_results(
        &self,
        results: Vec<MatchResult>,
        vector_storage: &crate::storage::MmapStorage,
        memo: &super::PayloadMemo<'_>,
        match_clause: &crate::velesql::MatchClause,
        query_vector: &[f32],
        expected_dimension: usize,
        metric: crate::distance::DistanceMetric,
        similarity_threshold: Option<f32>,
        higher_is_better: bool,
    ) -> Result<Vec<MatchResult>> {
        let mut scored_results = Vec::new();

        for mut result in results {
            if let Ok(Some(node_vector)) = vector_storage.retrieve(result.node_id) {
                validate_dimension_match(expected_dimension, node_vector.len())?;

                let score = metric.calculate(&node_vector, query_vector);

                if crate::velesql::match_threshold_keeps(
                    similarity_threshold,
                    score,
                    higher_is_better,
                ) {
                    result.score = Some(score);
                    result.projected = self.project_properties_with_score(
                        &result.bindings,
                        &result.edge_bindings,
                        &result.edge_paths,
                        &match_clause.return_clause,
                        Some(score),
                        memo,
                    );
                    scored_results.push(result);
                }
            }
        }

        Ok(scored_results)
    }

    /// Sorts scored results by similarity — descending for similarity metrics,
    /// ascending for distance metrics. A NaN or absent score ranks last, as in
    /// the browser executor.
    pub(super) fn sort_by_score(results: &mut [MatchResult], higher_is_better: bool) {
        let key = |r: &MatchResult| r.score.unwrap_or(f32::NAN);
        results.sort_by(|a, b| rank_scores(key(a), key(b), higher_is_better));
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
    // The guards back the memo the whole sort reads through.
    #[expect(clippy::significant_drop_tightening)]
    pub(super) fn order_match_results_by_property(
        &self,
        results: &mut [MatchResult],
        order_by: &str,
        descending: bool,
        sources: &super::MatchSources,
    ) -> Result<()> {
        let Some((alias, property)) = parse_property_path(order_by) else {
            return Err(crate::error::Error::GraphNotSupported(format!(
                "MATCH ORDER BY expression '{order_by}' is not supported \
                 (use similarity(), depth, or alias.property)"
            )));
        };
        // The annotated alias sorts on the properties WHERE and RETURN read,
        // through the same memo — which also reads each node once per query
        // instead of once per comparison.
        let reads = super::MatchReads::acquire(self, sources);
        let memo = super::PayloadMemo::with_annotations(&reads.payload, reads.annotations());
        results.sort_by(|a, b| {
            let get_value = |r: &MatchResult| -> Option<serde_json::Value> {
                let node_id = *r.bindings.get(alias)?;
                let payload = memo.get_bound(Some(alias), node_id);
                super::where_eval::element_property(node_id, property, |path| {
                    Self::get_nested_property(payload.as_deref()?.as_object()?, path)
                })
                .map(std::borrow::Cow::into_owned)
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
    // Both guards span the whole hydration loop: every iteration reads
    // payload and vector. Order is the post-mortem decree recorded below.
    #[expect(clippy::significant_drop_tightening)]
    pub fn match_results_to_search_results(
        &self,
        match_results: Vec<MatchResult>,
    ) -> Result<Vec<SearchResult>> {
        // LOCK ORDER: vector_storage(2) before payload_storage(3) — was
        // reversed here. See tests/tokio_blocking_lock_contention_repro.rs.
        let vector_storage = self.storage.vector_storage.read();
        let payload_storage = self.storage.payload_storage.read();

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

            // A `similarity()` score is the row's vector component, tagged
            // as a SELECT row's is; without one, depth is an inverse score
            // (closer = higher) and the row carries no component breakdown.
            let result = match mr.score {
                Some(similarity) => SearchResult::with_component_scores(
                    point,
                    similarity,
                    smallvec::smallvec![("vector_score", similarity)],
                ),
                None => SearchResult::new(point, 1.0 / (mr.depth as f32 + 1.0)),
            };
            results.push(result);
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
        let bindings = result
            .edge_bindings
            .iter()
            .map(|(alias, &id)| (alias.clone(), serde_id::id_to_json(id)))
            .collect();
        object.insert(
            "_edge_bindings".to_string(),
            serde_json::Value::Object(bindings),
        );
    }
    if !result.edge_paths.is_empty() {
        let paths = result
            .edge_paths
            .iter()
            .map(|(alias, ids)| (alias.clone(), serde_id::ids_to_json(ids)))
            .collect();
        object.insert("_edge_paths".to_string(), serde_json::Value::Object(paths));
    }
    serde_json::Value::Object(object)
}

/// A relationship's property as a MATCH row returns it: what WHERE reads,
/// except `id`, which leaves as its decimal string — a derived edge id is
/// hashed and routinely above 2^53, where a JSON number loses digits in
/// JavaScript. WHERE and ORDER BY keep comparing the number.
fn projected_edge_property(
    edge_id: u64,
    edge: &crate::collection::graph::GraphEdge,
    property: &str,
) -> Option<serde_json::Value> {
    if property == "id" {
        return Some(serde_id::id_to_json(edge_id));
    }
    super::where_eval::edge_property_path(edge, property).cloned()
}

#[cfg(test)]
#[path = "similarity_tests.rs"]
mod tests;
