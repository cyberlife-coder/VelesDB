//! VectorFirst MATCH execution strategy (Wave 6 Phase B).
//!
//! When the MATCH planner selects `VectorFirst`, the query is executed by:
//! 1. Running a vector similarity search to find top-k candidates
//! 2. Filtering candidates below the similarity threshold
//! 3. Validating each candidate against the graph pattern (BFS)
//! 4. Projecting RETURN properties with similarity scores
//!
//! This strategy is optimal when the start node has a similarity predicate
//! and the graph is dense (many edges per node), because the vector index
//! quickly narrows the candidate set before the more expensive graph check.

// Reason: f32->f64 for score injection into projected map; threshold comparisons
// use bounded similarity scores (0.0-1.0). No user-data truncation risk.
#![allow(clippy::cast_precision_loss)]

use super::where_eval::resolve_query_vector;
use super::MatchResult;
use crate::collection::graph::{concurrent_bfs_stream, StreamingConfig};
use crate::collection::types::Collection;
use crate::error::{Error, Result};
use crate::guardrails::QueryContext;
use crate::storage::PayloadStorage;
use crate::velesql::{Condition, MatchClause, SimilarityCondition};
use std::collections::HashMap;

impl Collection {
    /// Executes a MATCH query using the VectorFirst strategy.
    ///
    /// Finds top-k vector candidates, filters by similarity threshold,
    /// validates graph pattern existence for each candidate, then projects
    /// RETURN properties with similarity scores.
    ///
    /// # Errors
    ///
    /// Returns an error if the query vector cannot be resolved, the vector
    /// search fails, or graph validation encounters a storage error.
    pub(crate) fn execute_match_vector_first(
        &self,
        match_clause: &MatchClause,
        params: &HashMap<String, serde_json::Value>,
        ctx: &QueryContext,
        _similarity_alias: &str,
        top_k: usize,
        threshold: f32,
    ) -> Result<Vec<MatchResult>> {
        let sim_cond = extract_similarity_condition(match_clause.where_clause.as_ref())?;
        let query_vector = resolve_query_vector(&sim_cond.vector, params)?;

        if query_vector.is_empty() {
            return Ok(Vec::new());
        }

        let candidates = self.search(&query_vector, top_k)?;

        let config = self.config.read();
        let higher_is_better = config.metric.higher_is_better();
        drop(config);

        let above_threshold: Vec<_> = candidates
            .into_iter()
            .filter(|r| passes_threshold(r.score, threshold, higher_is_better))
            .collect();

        let limit = match_clause
            .return_clause
            .limit
            .and_then(|l| usize::try_from(l).ok())
            .unwrap_or(100);

        self.filter_candidates_by_graph(
            &above_threshold,
            match_clause,
            params,
            ctx,
            limit,
            higher_is_better,
        )
    }

    /// Validates each vector candidate against the graph pattern and builds
    /// `MatchResult` entries for those that pass.
    fn filter_candidates_by_graph(
        &self,
        candidates: &[crate::point::SearchResult],
        match_clause: &MatchClause,
        params: &HashMap<String, serde_json::Value>,
        ctx: &QueryContext,
        limit: usize,
        higher_is_better: bool,
    ) -> Result<Vec<MatchResult>> {
        let payload_guard = self.payload_storage.read();
        let mut results = Vec::new();

        for candidate in candidates {
            if results.len() >= limit {
                break;
            }

            ctx.check_timeout()
                .map_err(|e| Error::GuardRail(e.to_string()))?;

            let node_id = candidate.point.id;
            let Some(pattern) = match_clause.patterns.first() else {
                continue;
            };

            if !self.candidate_matches_start_pattern(node_id, pattern, &payload_guard) {
                continue;
            }

            let graph_ok = if pattern.relationships.is_empty() {
                self.candidate_passes_where(node_id, match_clause, params, &payload_guard, pattern)?
            } else {
                self.candidate_has_graph_path(
                    node_id,
                    match_clause,
                    params,
                    &payload_guard,
                    pattern,
                )?
            };

            if graph_ok {
                let mut mr = MatchResult::new(node_id, 0, Vec::new());
                mr.score = Some(candidate.score);

                if let Some(alias) = pattern.nodes.first().and_then(|n| n.alias.as_ref()) {
                    mr.bindings.insert(alias.clone(), node_id);
                }

                mr.projected = self.project_properties_with_score(
                    &mr.bindings,
                    &match_clause.return_clause,
                    Some(candidate.score),
                    &payload_guard,
                );

                results.push(mr);
            }
        }

        Self::sort_by_score(&mut results, higher_is_better);
        Ok(results)
    }

    /// Checks whether a candidate node matches the start node pattern
    /// (labels and properties) from the MATCH clause.
    #[allow(clippy::unused_self)] // Method on Collection for API consistency with start_nodes.rs
    fn candidate_matches_start_pattern(
        &self,
        node_id: u64,
        pattern: &crate::velesql::GraphPattern,
        payload_guard: &crate::storage::LogPayloadStorage,
    ) -> bool {
        let Some(first_node) = pattern.nodes.first() else {
            return true;
        };

        if first_node.labels.is_empty() && first_node.properties.is_empty() {
            return true;
        }

        let payload = payload_guard.retrieve(node_id).ok().flatten();

        if !first_node.labels.is_empty()
            && !Self::node_matches_labels(payload.as_ref(), &first_node.labels)
        {
            return false;
        }

        first_node.properties.is_empty()
            || Self::node_matches_properties(payload.as_ref(), &first_node.properties)
    }

    /// Validates that a candidate node passes the non-similarity WHERE conditions.
    ///
    /// Used when the MATCH pattern has no relationships (single-node pattern).
    fn candidate_passes_where(
        &self,
        node_id: u64,
        match_clause: &MatchClause,
        params: &HashMap<String, serde_json::Value>,
        payload_guard: &crate::storage::LogPayloadStorage,
        pattern: &crate::velesql::GraphPattern,
    ) -> Result<bool> {
        let non_sim = strip_similarity_from_where(match_clause.where_clause.as_ref());

        if let Some(ref cond) = non_sim {
            let mut bindings = HashMap::new();
            if let Some(alias) = pattern.nodes.first().and_then(|n| n.alias.as_ref()) {
                bindings.insert(alias.clone(), node_id);
            }
            self.evaluate_where_condition(node_id, Some(&bindings), cond, params, payload_guard)
        } else {
            Ok(true)
        }
    }

    /// Validates that a candidate node can reach at least one target through
    /// the graph relationships defined in the MATCH pattern.
    ///
    /// Runs a bounded BFS from `node_id` and checks the first hit against the
    /// non-similarity portion of the WHERE clause. Returns `true` if at least
    /// one valid path exists.
    fn candidate_has_graph_path(
        &self,
        node_id: u64,
        match_clause: &MatchClause,
        params: &HashMap<String, serde_json::Value>,
        payload_guard: &crate::storage::LogPayloadStorage,
        pattern: &crate::velesql::GraphPattern,
    ) -> Result<bool> {
        let max_depth = Self::compute_max_depth(pattern);
        let rel_types = Self::extract_rel_types(pattern);
        let non_sim = strip_similarity_from_where(match_clause.where_clause.as_ref());

        let config = StreamingConfig::default()
            .with_limit(1)
            .with_max_depth(max_depth)
            .with_rel_types(rel_types);

        let mut bindings = HashMap::new();
        if let Some(alias) = pattern.nodes.first().and_then(|n| n.alias.as_ref()) {
            bindings.insert(alias.clone(), node_id);
        }

        for hit in concurrent_bfs_stream(&self.edge_store, node_id, config) {
            if let Some(target_pattern) = pattern.nodes.get(hit.depth as usize) {
                if let Some(ref alias) = target_pattern.alias {
                    bindings.insert(alias.clone(), hit.target_id);
                }
            }

            if let Some(ref cond) = non_sim {
                if self.evaluate_where_condition(
                    hit.target_id,
                    Some(&bindings),
                    cond,
                    params,
                    payload_guard,
                )? {
                    return Ok(true);
                }
            } else {
                return Ok(true);
            }
        }

        Ok(false)
    }
}

/// Extracts the first `SimilarityCondition` from a WHERE clause tree.
///
/// Walks the condition recursively through AND/OR/NOT/Group wrappers.
///
/// # Errors
///
/// Returns `Error::Config` when no similarity condition is found.
fn extract_similarity_condition(where_clause: Option<&Condition>) -> Result<&SimilarityCondition> {
    fn find_sim(cond: &Condition) -> Option<&SimilarityCondition> {
        match cond {
            Condition::Similarity(sim) => Some(sim),
            Condition::And(l, r) | Condition::Or(l, r) => find_sim(l).or_else(|| find_sim(r)),
            Condition::Not(inner) | Condition::Group(inner) => find_sim(inner),
            _ => None,
        }
    }

    where_clause.and_then(find_sim).ok_or_else(|| {
        Error::Config(
            "VectorFirst strategy requires a similarity condition in WHERE clause".to_string(),
        )
    })
}

/// Returns the WHERE clause with the first `Similarity` leaf replaced by nothing.
///
/// When the entire WHERE is just a `Similarity`, returns `None`.
/// When `Similarity` is inside AND or OR, removes it and simplifies:
/// - `AND(sim, X)` or `AND(X, sim)` → `Some(X)` (both must hold; sim is
///   already satisfied by the vector search, so only X remains)
/// - `OR(sim, X)` or `OR(X, sim)` → `None` (either branch suffices; the
///   sim branch is already satisfied by the vector search, so the entire
///   OR is satisfied — no residual filter needed)
///
/// This allows the graph validation step to skip re-evaluating the similarity
/// condition (already handled by the vector search threshold filter).
fn strip_similarity_from_where(where_clause: Option<&Condition>) -> Option<Condition> {
    where_clause.and_then(strip_sim)
}

/// Recursively removes `Similarity` leaves from a condition tree.
///
/// Returns `None` when the condition itself is a `Similarity` leaf.
///
/// # Limitations
///
/// - Strips ALL `Similarity` leaves, not just the one handled by
///   vector search. Multiple similarity predicates in a single MATCH
///   WHERE clause are not supported by VectorFirst strategy.
/// - Does NOT recurse into `Not` nodes: `NOT(similarity(...) > 0.8)`
///   is a meaningful residual filter (reject high-similarity matches)
///   and must be preserved.
fn strip_sim(cond: &Condition) -> Option<Condition> {
    match cond {
        Condition::Similarity(_) => None,
        Condition::And(left, right) => match (strip_sim(left), strip_sim(right)) {
            (None, None) => None,
            (None, Some(r)) | (Some(r), None) => Some(r),
            (Some(l), Some(r)) => Some(Condition::And(Box::new(l), Box::new(r))),
        },
        Condition::Or(left, right) => match (strip_sim(left), strip_sim(right)) {
            // If either branch was the similarity condition, the entire OR
            // is satisfied by the vector search — no residual filter needed.
            (None | Some(_), None) | (None, Some(_)) => None,
            (Some(l), Some(r)) => Some(Condition::Or(Box::new(l), Box::new(r))),
        },
        // NOT(Similarity) is a meaningful residual: "reject matches above
        // threshold". Do not strip — preserve the entire NOT subtree.
        Condition::Not(_) => Some(cond.clone()),
        Condition::Group(inner) => strip_sim(inner).map(|c| Condition::Group(Box::new(c))),
        other => Some(other.clone()),
    }
}

/// Checks whether a score passes the similarity threshold.
///
/// For `higher_is_better` metrics (cosine, dot-product), the score must be
/// greater than or equal to the threshold. For distance metrics (Euclidean),
/// the score must be less than or equal.
fn passes_threshold(score: f32, threshold: f32, higher_is_better: bool) -> bool {
    if higher_is_better {
        score >= threshold
    } else {
        score <= threshold
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::velesql::{CompareOp, Comparison, SimilarityCondition, Value, VectorExpr};

    fn make_sim_condition() -> Condition {
        Condition::Similarity(SimilarityCondition {
            field: "doc.embedding".to_string(),
            vector: VectorExpr::Parameter("query".to_string()),
            operator: CompareOp::Gt,
            threshold: 0.8,
        })
    }

    fn make_comparison_condition() -> Condition {
        Condition::Comparison(Comparison {
            column: "category".to_string(),
            operator: CompareOp::Eq,
            value: Value::String("tech".to_string()),
        })
    }

    #[test]
    fn test_extract_similarity_condition_direct() {
        let sim = make_sim_condition();
        let extracted = extract_similarity_condition(Some(&sim)).expect("should find similarity");
        assert_eq!(extracted.field, "doc.embedding");
        assert!((extracted.threshold - 0.8).abs() < f64::EPSILON);
    }

    #[test]
    fn test_extract_similarity_condition_nested_and() {
        let cond = Condition::And(
            Box::new(make_comparison_condition()),
            Box::new(make_sim_condition()),
        );
        let extracted = extract_similarity_condition(Some(&cond)).expect("should find in AND");
        assert_eq!(extracted.field, "doc.embedding");
    }

    #[test]
    fn test_extract_similarity_condition_missing() {
        let cond = make_comparison_condition();
        let result = extract_similarity_condition(Some(&cond));
        assert!(result.is_err(), "should fail when no similarity present");
    }

    #[test]
    fn test_extract_similarity_condition_none_where() {
        let result = extract_similarity_condition(None);
        assert!(result.is_err(), "should fail on None where clause");
    }

    #[test]
    fn test_strip_similarity_removes_direct() {
        let sim = make_sim_condition();
        let stripped = strip_similarity_from_where(Some(&sim));
        assert!(stripped.is_none(), "bare similarity should strip to None");
    }

    #[test]
    fn test_strip_similarity_keeps_other_in_and() {
        let cmp = make_comparison_condition();
        let sim = make_sim_condition();
        let cond = Condition::And(Box::new(sim), Box::new(cmp.clone()));
        let stripped = strip_similarity_from_where(Some(&cond));
        assert!(stripped.is_some(), "AND(sim, cmp) should keep cmp");
        assert!(
            matches!(stripped, Some(Condition::Comparison(_))),
            "result should be the comparison"
        );
    }

    #[test]
    fn test_strip_similarity_drops_entire_or() {
        let cmp = make_comparison_condition();
        let sim = make_sim_condition();
        let cond = Condition::Or(Box::new(cmp.clone()), Box::new(sim));
        let stripped = strip_similarity_from_where(Some(&cond));
        assert!(
            stripped.is_none(),
            "OR(cmp, sim) should be None — the similarity branch satisfies the entire OR"
        );
    }

    #[test]
    fn test_strip_similarity_preserves_or_without_sim() {
        let cmp1 = make_comparison_condition();
        let cmp2 = Condition::Comparison(Comparison {
            column: "price".to_string(),
            operator: CompareOp::Gt,
            value: Value::Float(42.0),
        });
        let cond = Condition::Or(Box::new(cmp1), Box::new(cmp2));
        let stripped = strip_similarity_from_where(Some(&cond));
        assert!(
            matches!(stripped, Some(Condition::Or(..))),
            "OR(cmp1, cmp2) with no similarity should preserve both branches"
        );
    }

    #[test]
    fn test_strip_similarity_preserves_non_sim_tree() {
        let cmp = make_comparison_condition();
        let stripped = strip_similarity_from_where(Some(&cmp));
        assert!(
            matches!(stripped, Some(Condition::Comparison(_))),
            "non-similarity condition should pass through"
        );
    }

    #[test]
    fn test_passes_threshold_higher_is_better() {
        assert!(passes_threshold(0.9, 0.8, true));
        assert!(passes_threshold(0.8, 0.8, true));
        assert!(!passes_threshold(0.7, 0.8, true));
    }

    #[test]
    fn test_passes_threshold_lower_is_better() {
        assert!(passes_threshold(0.3, 0.5, false));
        assert!(passes_threshold(0.5, 0.5, false));
        assert!(!passes_threshold(0.7, 0.5, false));
    }

    // Regression test: Devin review — NOT(Similarity) must be preserved.
    // Stripping NOT(sim) inverts query semantics: "reject high-similarity"
    // becomes "no filter at all".

    #[test]
    fn test_strip_sim_preserves_not_similarity() {
        let sim = make_sim_condition();
        let cond = Condition::Not(Box::new(sim));
        let stripped = strip_similarity_from_where(Some(&cond));
        assert!(
            matches!(stripped, Some(Condition::Not(_))),
            "NOT(Similarity) must be preserved — it is a meaningful residual filter"
        );
    }

    #[test]
    fn test_strip_sim_preserves_not_non_sim() {
        let cmp = make_comparison_condition();
        let cond = Condition::Not(Box::new(cmp));
        let stripped = strip_similarity_from_where(Some(&cond));
        assert!(
            matches!(stripped, Some(Condition::Not(_))),
            "NOT(comparison) should always be preserved"
        );
    }
}
