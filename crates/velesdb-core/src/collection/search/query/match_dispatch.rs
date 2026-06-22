//! Internal dispatch helpers for MATCH query execution.
//!
//! Extracted from `select_dispatch.rs` (Martin Fowler: Extract Module) to keep
//! file NLOC under 500. These methods handle MATCH dispatch, parallel
//! execution, result merging, and MATCH-specific metrics.

use crate::collection::graph::property_index::PredicateType;
use crate::collection::types::Collection;
use crate::error::Result;
use crate::point::SearchResult;
use crate::velesql::{CompareOp, Condition};

use super::MAX_LIMIT;

/// Global MATCH query metrics collector (EPIC-050).
///
/// Uses `LazyLock` for thread-safe one-time initialisation.
/// Per-collection metrics registries are a future enhancement.
static MATCH_METRICS: std::sync::LazyLock<super::match_metrics::MatchMetrics> =
    std::sync::LazyLock::new(super::match_metrics::MatchMetrics::new);

impl Collection {
    /// Computes collection statistics for MATCH query planning.
    ///
    /// Gathers node count, edge count, average degree, and label statistics
    /// from the live collection data structures for cost-based strategy selection.
    // Reason: usize->f64 casts are for cost-estimation ratios, not precise calculations.
    #[allow(clippy::cast_precision_loss)]
    fn compute_match_collection_stats(&self) -> super::match_planner::CollectionStats {
        let total_nodes = self.len();
        let total_edges = self.edge_store.len();
        let avg_degree = if total_nodes > 0 {
            total_edges as f64 / total_nodes as f64
        } else {
            0.0
        };
        let label_count = self.edge_store.label_count();
        let label_selectivity = if label_count > 0 {
            1.0 / label_count as f64
        } else {
            1.0
        };
        super::match_planner::CollectionStats {
            total_nodes,
            total_edges,
            avg_degree,
            label_count,
            label_selectivity,
        }
    }

    /// Dispatches a MATCH query through the graph traversal path.
    ///
    /// Calls the cost-based `MatchQueryPlanner` to select an execution strategy,
    /// records query metrics via the global `MATCH_METRICS` collector, then
    /// delegates to the graph traversal engine.
    pub(super) fn dispatch_match_query(
        &self,
        match_clause: &crate::velesql::MatchClause,
        params: &std::collections::HashMap<String, serde_json::Value>,
        ctx: &crate::guardrails::QueryContext,
    ) -> Result<Vec<SearchResult>> {
        let start = std::time::Instant::now();

        // W6-A2: Cost-based strategy selection.
        let stats = self.compute_match_collection_stats();
        let strategy = super::match_planner::MatchQueryPlanner::plan(match_clause, &stats);
        tracing::debug!(strategy = ?strategy, "MATCH execution strategy selected");

        // Dispatch based on strategy.
        let result = match &strategy {
            super::match_planner::MatchExecutionStrategy::VectorFirst {
                similarity_alias,
                top_k,
                threshold,
            } => {
                let vf_results = self.execute_match_vector_first(
                    match_clause,
                    params,
                    ctx,
                    similarity_alias,
                    *top_k,
                    *threshold,
                )?;
                self.finalize_match_results(match_clause, vf_results, ctx, params)
            }
            super::match_planner::MatchExecutionStrategy::Parallel {
                ref vector_hint, ..
            } => self.execute_match_parallel(match_clause, params, ctx, vector_hint),
            super::match_planner::MatchExecutionStrategy::GraphFirst { .. } => {
                self.execute_match_pipeline(match_clause, params, ctx)
            }
        };

        // W6-A3: Record metrics.
        let max_depth = super::match_planner::MatchQueryPlanner::count_hops(match_clause);
        match &result {
            Ok(results) => {
                MATCH_METRICS.record_success(start.elapsed(), results.len(), max_depth);
            }
            Err(_) => {
                MATCH_METRICS.record_failure(start.elapsed());
            }
        }

        // S4-10: Record query pattern for the index advisor.
        if result.is_ok() {
            // Reason: u128->u64 cast; query durations < u64::MAX ms (~585 millennia)
            #[allow(clippy::cast_possible_truncation)]
            let elapsed_ms = start.elapsed().as_millis() as u64;
            let (labels, properties, predicates) = extract_match_query_pattern(match_clause);
            self.record_query_pattern(labels, properties, predicates, elapsed_ms);
        }

        result
    }

    /// Executes the MATCH pipeline: traversal, ordering, conversion, and limits.
    ///
    /// Factored out of `dispatch_match_query` so metrics recording wraps the
    /// entire operation cleanly.
    fn execute_match_pipeline(
        &self,
        match_clause: &crate::velesql::MatchClause,
        params: &std::collections::HashMap<String, serde_json::Value>,
        ctx: &crate::guardrails::QueryContext,
    ) -> Result<Vec<SearchResult>> {
        let match_results = self.execute_match_with_context(match_clause, params, Some(ctx))?;
        self.finalize_match_results(match_clause, match_results, ctx, params)
    }

    /// Executes the Parallel MATCH strategy (Wave 6 Phase D).
    ///
    /// Runs GraphFirst and VectorFirst sequentially, then merges the result
    /// sets by `node_id` (union semantics -- best score wins for duplicates).
    ///
    /// True parallel execution (rayon/tokio) is a future optimisation; the
    /// sequential approach is correct and avoids concurrency complexity for
    /// typical MATCH query sizes.
    fn execute_match_parallel(
        &self,
        match_clause: &crate::velesql::MatchClause,
        params: &std::collections::HashMap<String, serde_json::Value>,
        ctx: &crate::guardrails::QueryContext,
        vector_hint: &super::match_planner::MatchExecutionStrategy,
    ) -> Result<Vec<SearchResult>> {
        // Phase 1: GraphFirst path.
        let graph_results = self.execute_match_with_context(match_clause, params, Some(ctx))?;

        // Phase 2: VectorFirst path (extract hint parameters).
        let vector_results = if let super::match_planner::MatchExecutionStrategy::VectorFirst {
            similarity_alias,
            top_k,
            threshold,
        } = vector_hint
        {
            self.execute_match_vector_first(
                match_clause,
                params,
                ctx,
                similarity_alias,
                *top_k,
                *threshold,
            )?
        } else {
            tracing::warn!(
                "Parallel strategy vector_hint is not VectorFirst; \
                     skipping vector path"
            );
            Vec::new()
        };

        // Phase 3: Merge by node_id (union, best score wins per metric polarity).
        let config = self.config.read();
        let higher_is_better = config.metric.higher_is_better();
        drop(config);

        let merged = merge_match_results(graph_results, vector_results, higher_is_better);
        self.finalize_match_results(match_clause, merged, ctx, params)
    }

    /// Applies ORDER BY, conversion to `SearchResult`, cardinality check,
    /// LIMIT, and latency recording to a set of `MatchResult`s.
    ///
    /// Shared by GraphFirst, VectorFirst, and Parallel strategies.
    fn finalize_match_results(
        &self,
        match_clause: &crate::velesql::MatchClause,
        match_results: Vec<super::match_exec::MatchResult>,
        ctx: &crate::guardrails::QueryContext,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<Vec<SearchResult>> {
        ctx.check_timeout()
            .map_err(crate::error::Error::from)
            .inspect_err(|_| self.guard_rails.circuit_breaker.record_failure())?;

        let mut sorted = match_results;
        if let Some(order_by) = match_clause.return_clause.order_by.as_ref() {
            for item in order_by.iter().rev() {
                self.order_match_results(&mut sorted, &item.expr, item.descending, params)
                    .inspect_err(|_| self.guard_rails.circuit_breaker.record_failure())?;
            }
        }

        let mut results = self
            .match_results_to_search_results(sorted)
            .inspect_err(|_| self.guard_rails.circuit_breaker.record_failure())?;
        // Final cardinality check for MATCH path (EPIC-048 US-003).
        ctx.check_cardinality(results.len())
            .map_err(crate::error::Error::from)
            .inspect_err(|_| self.guard_rails.circuit_breaker.record_failure())?;
        if let Some(limit) = match_clause.return_clause.limit {
            let limit = usize::try_from(limit).unwrap_or(MAX_LIMIT).min(MAX_LIMIT);
            results.truncate(limit);
        }
        // Reason: u128->u64 cast; query durations < u64::MAX µs (~585 millennia)
        #[allow(clippy::cast_possible_truncation)]
        let graph_latency_us = ctx.elapsed().as_micros() as u64;
        self.query_planner
            .stats()
            .update_graph_latency(graph_latency_us);
        self.guard_rails.circuit_breaker.record_success();
        Ok(results)
    }
}

/// Extracts labels, property names, and predicate types from a MATCH clause
/// for index advisor pattern tracking (S4-10).
///
/// Labels come from all `NodePattern.labels` across every pattern.
/// Properties and predicates come from the WHERE clause conditions.
fn extract_match_query_pattern(
    match_clause: &crate::velesql::MatchClause,
) -> (Vec<String>, Vec<String>, Vec<PredicateType>) {
    let mut labels: Vec<String> = match_clause
        .patterns
        .iter()
        .flat_map(|p| p.nodes.iter())
        .flat_map(|n| n.labels.iter())
        .cloned()
        .collect();
    labels.sort_unstable();
    labels.dedup();

    let mut properties: Vec<String> = Vec::new();
    let mut predicates: Vec<PredicateType> = Vec::new();

    if let Some(ref cond) = match_clause.where_clause {
        collect_condition_predicates(cond, &mut properties, &mut predicates);
    }

    properties.sort_unstable();
    properties.dedup();

    (labels, properties, predicates)
}

/// Recursively walks a `Condition` tree and collects property names and
/// their corresponding `PredicateType` for the index advisor.
// Reason: Condition is #[non_exhaustive] — the wildcard arm is required for
// forward-compatibility when new variants are added, even though the compiler
// currently sees all arms as covered within the same crate.
#[allow(unreachable_patterns)]
fn collect_condition_predicates(
    cond: &Condition,
    properties: &mut Vec<String>,
    predicates: &mut Vec<PredicateType>,
) {
    match cond {
        Condition::Comparison(c) => {
            properties.push(c.column.clone());
            let pred = match c.operator {
                CompareOp::Eq | CompareOp::NotEq => PredicateType::Equality,
                CompareOp::Gt | CompareOp::Gte | CompareOp::Lt | CompareOp::Lte => {
                    PredicateType::Range
                }
            };
            predicates.push(pred);
        }
        Condition::In(i) => {
            properties.push(i.column.clone());
            predicates.push(PredicateType::In);
        }
        Condition::Between(b) => {
            properties.push(b.column.clone());
            predicates.push(PredicateType::Range);
        }
        Condition::Like(l) => {
            properties.push(l.column.clone());
            predicates.push(PredicateType::Like);
        }
        Condition::And(lhs, rhs) | Condition::Or(lhs, rhs) => {
            collect_condition_predicates(lhs, properties, predicates);
            collect_condition_predicates(rhs, properties, predicates);
        }
        Condition::Not(inner) | Condition::Group(inner) => {
            collect_condition_predicates(inner, properties, predicates);
        }
        // All remaining variants (vector search, similarity, null checks,
        // full-text match, graph match, contains, geo conditions, and any
        // future #[non_exhaustive] additions) do not map to property index
        // predicates — intentionally skipped.
        _ => {}
    }
}

/// Merges the GraphFirst and VectorFirst result sets (union semantics).
///
/// Graph rows are authoritative row identities: the pattern walker already
/// deduplicates them by full binding signature, so every graph row — one per
/// aliased parallel edge or distinct edge path — is kept. A vector row is
/// node-level enrichment: when graph rows exist for its `node_id`, it merges
/// its (similarity) score and missing data into **every** row of that node
/// (the score describes the node's embedding, not one edge); otherwise it
/// stands alone as the node's row (union). The better score wins per row
/// (higher for similarity metrics, lower for distance metrics); rows without
/// a score use a sentinel that always loses to real scores.
///
/// Audit 2026-06 F2: replacing whole entries dropped plan-specific data — a
/// GraphFirst row's `r.*` projection/edge bindings were clobbered by the
/// VectorFirst candidate for the same `node_id`. Enrichment keeps every
/// graph row and only fills in (or score-overrides) what the vector row
/// contributes. Review 2026-06-11: enrichment applies to ALL rows of the
/// node group, so parallel-edge siblings rank by the same node score instead
/// of one arbitrary row absorbing it.
///
/// The merged output is sorted best-to-worst according to `higher_is_better`.
fn merge_match_results(
    graph_results: Vec<super::match_exec::MatchResult>,
    vector_results: Vec<super::match_exec::MatchResult>,
    higher_is_better: bool,
) -> Vec<super::match_exec::MatchResult> {
    use std::collections::HashMap;

    let mut by_node: HashMap<u64, Vec<super::match_exec::MatchResult>> =
        HashMap::with_capacity(graph_results.len() + vector_results.len());
    for row in graph_results {
        by_node.entry(row.node_id).or_default().push(row);
    }

    for candidate in vector_results {
        match by_node.entry(candidate.node_id) {
            std::collections::hash_map::Entry::Occupied(mut group) => {
                for row in group.get_mut() {
                    enrich_row(row, &candidate, higher_is_better);
                }
            }
            std::collections::hash_map::Entry::Vacant(slot) => {
                slot.insert(vec![candidate]);
            }
        }
    }

    let mut merged: Vec<super::match_exec::MatchResult> = by_node.into_values().flatten().collect();
    sort_match_results_by_score(&mut merged, higher_is_better);
    merged
}

/// Enriches one graph row with a vector candidate for the same node.
///
/// When the candidate's score is better, it replaces the row's score and its
/// data takes priority on shared keys (e.g. a fresher `similarity()`
/// projection); otherwise the candidate only fills keys the row lacks.
fn enrich_row(
    row: &mut super::match_exec::MatchResult,
    candidate: &super::match_exec::MatchResult,
    higher_is_better: bool,
) {
    let worse_sentinel = if higher_is_better {
        f32::NEG_INFINITY
    } else {
        f32::MAX
    };
    let candidate_score = candidate.score.unwrap_or(worse_sentinel);
    let row_score = row.score.unwrap_or(worse_sentinel);
    let candidate_wins = if higher_is_better {
        candidate_score > row_score
    } else {
        candidate_score < row_score
    };
    if candidate_wins {
        row.score = candidate.score;
    }
    merge_map(&mut row.projected, &candidate.projected, candidate_wins);
    merge_map(&mut row.bindings, &candidate.bindings, candidate_wins);
    merge_map(
        &mut row.edge_bindings,
        &candidate.edge_bindings,
        candidate_wins,
    );
    merge_map(&mut row.edge_paths, &candidate.edge_paths, candidate_wins);
}

/// Copies `source` entries into `target`: overwriting on shared keys when
/// `source_wins`, otherwise only filling keys the target lacks.
fn merge_map<V: Clone>(
    target: &mut std::collections::HashMap<String, V>,
    source: &std::collections::HashMap<String, V>,
    source_wins: bool,
) {
    for (key, value) in source {
        if source_wins {
            target.insert(key.clone(), value.clone());
        } else {
            target.entry(key.clone()).or_insert_with(|| value.clone());
        }
    }
}

/// Sorts `merged` by score using the same polarity-aware logic as `sort_by_score` in `similarity.rs`.
fn sort_match_results_by_score(
    merged: &mut [super::match_exec::MatchResult],
    higher_is_better: bool,
) {
    if higher_is_better {
        merged.sort_unstable_by(|a, b| {
            let sa = a.score.unwrap_or(f32::NEG_INFINITY);
            let sb = b.score.unwrap_or(f32::NEG_INFINITY);
            sb.total_cmp(&sa)
        });
    } else {
        merged.sort_unstable_by(|a, b| {
            let sa = a.score.unwrap_or(f32::MAX);
            let sb = b.score.unwrap_or(f32::MAX);
            sa.total_cmp(&sb)
        });
    }
}

#[cfg(test)]
mod tests {
    use super::super::match_exec::MatchResult;
    use super::merge_match_results;

    fn mr(node_id: u64, score: Option<f32>) -> MatchResult {
        let mut r = MatchResult::new(node_id, 0, Vec::new());
        r.score = score;
        r
    }

    // --- higher_is_better = true (cosine / dot-product) ---

    #[test]
    fn test_merge_empty_inputs() {
        let merged = merge_match_results(Vec::new(), Vec::new(), true);
        assert!(merged.is_empty());
    }

    #[test]
    fn test_merge_graph_only() {
        let graph = vec![mr(1, None), mr(2, Some(0.5))];
        let merged = merge_match_results(graph, Vec::new(), true);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].node_id, 2);
    }

    #[test]
    fn test_merge_vector_only() {
        let vector = vec![mr(3, Some(0.9)), mr(4, Some(0.7))];
        let merged = merge_match_results(Vec::new(), vector, true);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].node_id, 3);
        assert_eq!(merged[1].node_id, 4);
    }

    #[test]
    fn test_merge_union_distinct_nodes() {
        let graph = vec![mr(1, None), mr(2, None)];
        let vector = vec![mr(3, Some(0.8)), mr(4, Some(0.6))];
        let merged = merge_match_results(graph, vector, true);
        assert_eq!(merged.len(), 4);
    }

    #[test]
    fn test_merge_duplicate_keeps_higher_score() {
        let graph = vec![mr(1, Some(0.3))];
        let vector = vec![mr(1, Some(0.9))];
        let merged = merge_match_results(graph, vector, true);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].node_id, 1);
        assert!((merged[0].score.expect("test: should have score") - 0.9).abs() < f32::EPSILON);
    }

    #[test]
    fn test_merge_duplicate_graph_wins_when_higher() {
        let graph = vec![mr(1, Some(0.95))];
        let vector = vec![mr(1, Some(0.5))];
        let merged = merge_match_results(graph, vector, true);
        assert_eq!(merged.len(), 1);
        assert!((merged[0].score.expect("test: should have score") - 0.95).abs() < f32::EPSILON);
    }

    #[test]
    fn test_merge_sorted_descending() {
        let graph = vec![mr(1, Some(0.3)), mr(2, Some(0.1))];
        let vector = vec![mr(3, Some(0.9)), mr(4, Some(0.5))];
        let merged = merge_match_results(graph, vector, true);
        let scores: Vec<f32> = merged
            .iter()
            .map(|r| r.score.unwrap_or(f32::NEG_INFINITY))
            .collect();
        for w in scores.windows(2) {
            assert!(w[0] >= w[1], "scores should be descending: {scores:?}");
        }
    }

    #[test]
    fn test_merge_none_scores_sorted_last() {
        let graph = vec![mr(1, None), mr(2, None)];
        let vector = vec![mr(3, Some(0.5))];
        let merged = merge_match_results(graph, vector, true);
        assert_eq!(merged.len(), 3);
        assert_eq!(merged[0].node_id, 3);
    }

    // --- higher_is_better = false (euclidean / hamming) ---

    #[test]
    fn test_merge_euclidean_duplicate_keeps_lower_score() {
        let graph = vec![mr(1, Some(0.9))];
        let vector = vec![mr(1, Some(0.2))];
        let merged = merge_match_results(graph, vector, false);
        assert_eq!(merged.len(), 1);
        assert!(
            (merged[0].score.expect("test: should have score") - 0.2).abs() < f32::EPSILON,
            "Euclidean: lower distance should win"
        );
    }

    #[test]
    fn test_merge_euclidean_graph_wins_when_lower() {
        let graph = vec![mr(1, Some(0.1))];
        let vector = vec![mr(1, Some(0.8))];
        let merged = merge_match_results(graph, vector, false);
        assert_eq!(merged.len(), 1);
        assert!(
            (merged[0].score.expect("test: should have score") - 0.1).abs() < f32::EPSILON,
            "Euclidean: graph result with lower distance should win"
        );
    }

    #[test]
    fn test_merge_euclidean_sorted_ascending() {
        let graph = vec![mr(1, Some(0.9)), mr(2, Some(0.3))];
        let vector = vec![mr(3, Some(0.1)), mr(4, Some(0.5))];
        let merged = merge_match_results(graph, vector, false);
        let scores: Vec<f32> = merged.iter().map(|r| r.score.unwrap_or(f32::MAX)).collect();
        for w in scores.windows(2) {
            assert!(
                w[0] <= w[1],
                "Euclidean scores should be ascending (best first): {scores:?}"
            );
        }
    }

    #[test]
    fn test_merge_euclidean_none_scores_sorted_last() {
        let graph = vec![mr(1, None), mr(2, None)];
        let vector = vec![mr(3, Some(0.5))];
        let merged = merge_match_results(graph, vector, false);
        assert_eq!(merged.len(), 3);
        assert_eq!(
            merged[0].node_id, 3,
            "Euclidean: scored result should sort before None"
        );
    }

    #[test]
    fn test_merge_empty_inputs_euclidean() {
        let merged = merge_match_results(Vec::new(), Vec::new(), false);
        assert!(merged.is_empty());
    }

    // --- collision data merge (audit 2026-06 cluster F2, finding 5) ---

    /// Builds a GraphFirst-style result: unscored, with edge projection data.
    fn graph_mr_with_edge_data(node_id: u64) -> MatchResult {
        let mut r = MatchResult::new(node_id, 1, vec![100]);
        r.bindings.insert("b".to_string(), node_id);
        r.edge_bindings.insert("r".to_string(), 100);
        r.projected
            .insert("r.since".to_string(), serde_json::json!(2020));
        r
    }

    /// GIVEN a GraphFirst result carrying `r.since` projection + edge binding
    ///   and a scored VectorFirst candidate for the same node without them
    /// WHEN the candidate wins the score comparison
    /// THEN the winning score is kept BUT the GraphFirst-only projection,
    ///      edge bindings, and node bindings survive the merge.
    #[test]
    fn test_merge_collision_preserves_graph_edge_data() {
        let graph = vec![graph_mr_with_edge_data(1)];
        let vector = vec![mr(1, Some(0.9))];

        let merged = merge_match_results(graph, vector, true);

        assert_eq!(merged.len(), 1);
        assert!(
            (merged[0].score.expect("test: should have score") - 0.9).abs() < f32::EPSILON,
            "the better (vector) score must win"
        );
        assert_eq!(
            merged[0].projected.get("r.since"),
            Some(&serde_json::json!(2020)),
            "GraphFirst projection must survive the collision merge"
        );
        assert_eq!(
            merged[0].edge_bindings.get("r"),
            Some(&100),
            "GraphFirst edge binding must survive the collision merge"
        );
        assert_eq!(
            merged[0].bindings.get("b"),
            Some(&1),
            "GraphFirst node binding must survive the collision merge"
        );
    }

    /// GIVEN a scored GraphFirst result that beats the vector candidate
    /// WHEN the candidate loses the score comparison
    /// THEN candidate-only data (e.g. its projection keys) still survives.
    #[test]
    fn test_merge_collision_preserves_loser_only_keys() {
        let mut graph = graph_mr_with_edge_data(1);
        graph.score = Some(0.95);
        let mut vector = mr(1, Some(0.5));
        vector
            .projected
            .insert("similarity()".to_string(), serde_json::json!(0.5));

        let merged = merge_match_results(vec![graph], vec![vector], true);

        assert_eq!(merged.len(), 1);
        assert!(
            (merged[0].score.expect("test: should have score") - 0.95).abs() < f32::EPSILON,
            "the better (graph) score must win"
        );
        assert!(
            merged[0].projected.contains_key("similarity()"),
            "loser-only projection keys must survive the collision merge"
        );
        assert_eq!(
            merged[0].projected.get("r.since"),
            Some(&serde_json::json!(2020)),
            "winner projection must be untouched"
        );
    }

    /// GIVEN two parallel-edge graph rows for the same node (distinct edge
    ///   bindings) and one scored vector candidate for that node
    /// WHEN the Parallel strategy merges the result sets
    /// THEN BOTH rows survive AND both carry the node-level score (review
    ///      2026-06-11: enrichment must reach every row of the node group,
    ///      not the first one found).
    #[test]
    fn test_merge_enriches_all_parallel_edge_rows() {
        let mut g1 = graph_mr_with_edge_data(1);
        g1.edge_bindings.insert("r".to_string(), 100);
        let mut g2 = graph_mr_with_edge_data(1);
        g2.edge_bindings.insert("r".to_string(), 101);
        let vector = vec![mr(1, Some(0.9))];

        let merged = merge_match_results(vec![g1, g2], vector, true);

        assert_eq!(merged.len(), 2, "both parallel-edge rows must survive");
        for row in &merged {
            assert!(
                (row.score.expect("test: enriched score") - 0.9).abs() < f32::EPSILON,
                "every row of the node group must carry the node-level score"
            );
        }
        let mut edge_ids: Vec<u64> = merged
            .iter()
            .filter_map(|r| r.edge_bindings.get("r").copied())
            .collect();
        edge_ids.sort_unstable();
        assert_eq!(edge_ids, vec![100, 101], "edge identities must be distinct");
    }
}
