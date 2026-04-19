//! Plan construction logic for `VelesQL` EXPLAIN.
//!
//! Contains `impl QueryPlan` methods for building plans from SELECT statements,
//! MATCH clauses, and related query structures.

use std::collections::HashSet;

use super::formatter;
use super::node_stats;
use super::types::{
    FilterPlan, FilterStrategy, FusionInfo, IndexLookupPlan, IndexType, LimitPlan,
    MatchTraversalPlan, OffsetPlan, PlanNode, QueryPlan, TableScanPlan, VectorSearchPlan,
};
use crate::collection::search::query::match_planner::{
    CollectionStats, MatchExecutionStrategy, MatchQueryPlanner,
};
use crate::collection::stats::CollectionStats as CoreCollectionStats;
use crate::velesql::ast::{Condition, LetBinding, SelectStatement};
use crate::velesql::cost_estimator::CostEstimator;
use crate::velesql::MatchClause;

/// Threshold (selectivity) above which the fallback heuristic switches from
/// PreFilter to PostFilter. Preserved unchanged for backward compatibility
/// with tests that predate calibrated costs (see
/// `test_filter_strategy_post_filter_default`).
const FALLBACK_SELECTIVITY_THRESHOLD: f64 = 0.1;

/// Guardrail: if a predicate filters out less than this fraction of rows,
/// running the pre-filter before HNSW is likely to hurt recall (too many
/// candidates survive). Forces PostFilter in that case.
const PREFILTER_RECALL_GUARD: f64 = 0.5;

impl QueryPlan {
    /// Creates a new query plan from a SELECT statement.
    #[must_use]
    pub fn from_select(stmt: &SelectStatement) -> Self {
        Self::from_select_with_stats(stmt, &HashSet::new(), None)
    }

    /// Creates a new query plan from SELECT with known indexed metadata fields.
    #[must_use]
    pub fn from_select_with_indexed_fields(
        stmt: &SelectStatement,
        indexed_fields: &HashSet<String>,
    ) -> Self {
        Self::from_select_with_stats(stmt, indexed_fields, None)
    }

    /// Creates a query plan with access to calibrated collection statistics.
    ///
    /// When `stats` is `Some`, cost and filter-strategy decisions use the
    /// calibrated `CostEstimator` pipeline (issue #471). When `None`, falls
    /// back bit-for-bit to the heuristic path so legacy tests and callers
    /// without a resolved collection keep working.
    #[must_use]
    pub fn from_select_with_stats(
        stmt: &SelectStatement,
        indexed_fields: &HashSet<String>,
        stats: Option<&CoreCollectionStats>,
    ) -> Self {
        let mut has_vector_search = false;
        let mut filter_conditions = Vec::new();
        let mut index_lookup = None;

        if let Some(ref condition) = stmt.where_clause {
            Self::analyze_condition(condition, &mut has_vector_search, &mut filter_conditions);
            index_lookup = Self::extract_index_lookup(condition, indexed_fields);
        }

        let (mut nodes, index_used) = Self::build_scan_node(stmt, has_vector_search, index_lookup);
        let filter_strategy = Self::append_filter_nodes_with_stats(
            &mut nodes,
            &filter_conditions,
            stmt,
            has_vector_search,
            stats,
        );

        let mut plan = Self::assemble_plan_with_stats(
            nodes,
            index_used,
            filter_strategy,
            has_vector_search,
            stats,
        );
        plan.with_options = Self::extract_with_options(stmt);
        plan.fusion_info = Self::extract_fusion_info(stmt);
        plan
    }

    /// Creates a full query plan from a `Query`, including LET bindings (issue #471).
    #[must_use]
    pub fn from_query(query: &crate::velesql::ast::Query) -> Self {
        Self::from_query_with_stats(query, &HashSet::new(), None)
    }

    /// Creates a full query plan from a `Query`, with optional collection stats
    /// for calibrated cost estimation (issue #471).
    #[must_use]
    pub fn from_query_with_stats(
        query: &crate::velesql::ast::Query,
        indexed_fields: &HashSet<String>,
        stats: Option<&CoreCollectionStats>,
    ) -> Self {
        let mut plan = Self::from_select_with_stats(&query.select, indexed_fields, stats);
        plan.let_bindings = Self::format_let_bindings(&query.let_bindings);
        plan
    }

    /// Creates a new query plan from a MATCH clause (EPIC-046 US-004).
    #[must_use]
    pub fn from_match(match_clause: &MatchClause, stats: &CollectionStats) -> Self {
        let strategy = MatchQueryPlanner::plan(match_clause, stats);
        let strategy_explanation = MatchQueryPlanner::explain(&strategy);

        let (start_labels, max_depth, has_similarity, similarity_threshold) =
            Self::extract_strategy_info(&strategy);

        let relationship_count = match_clause
            .patterns
            .first()
            .map_or(0, |p| p.relationships.len());

        let traversal = PlanNode::MatchTraversal(MatchTraversalPlan {
            strategy: strategy_explanation,
            start_labels,
            max_depth,
            relationship_count,
            has_similarity,
            similarity_threshold,
        });

        let mut nodes = vec![traversal];
        if let Some(limit) = match_clause.return_clause.limit {
            nodes.push(PlanNode::Limit(LimitPlan { count: limit }));
        }

        let index_used = if has_similarity {
            Some(IndexType::Hnsw)
        } else {
            None
        };

        Self::assemble_plan_with_stats(
            nodes,
            index_used,
            FilterStrategy::None,
            has_similarity,
            None,
        )
    }

    /// Variant of `assemble_plan` with optional calibrated `CollectionStats`.
    fn assemble_plan_with_stats(
        mut nodes: Vec<PlanNode>,
        index_used: Option<IndexType>,
        filter_strategy: FilterStrategy,
        has_vector_search: bool,
        stats: Option<&CoreCollectionStats>,
    ) -> Self {
        let root = if nodes.len() == 1 {
            nodes.swap_remove(0)
        } else {
            PlanNode::Sequence(nodes)
        };
        let estimated_cost_ms = node_stats::estimate_cost(&root, has_vector_search, stats);
        Self {
            root,
            estimated_cost_ms,
            index_used,
            filter_strategy,
            with_options: Vec::new(),
            let_bindings: Vec::new(),
            fusion_info: None,
            cache_hit: None,
            plan_reuse_count: None,
        }
    }

    /// Default `ef_search` when the WITH clause does not specify one.
    const DEFAULT_EF_SEARCH: u32 = 100;

    /// Builds the primary scan node based on search type.
    fn build_scan_node(
        stmt: &SelectStatement,
        has_vector_search: bool,
        index_lookup: Option<(String, String)>,
    ) -> (Vec<PlanNode>, Option<IndexType>) {
        let mut nodes = Vec::new();
        let index_used;

        if has_vector_search {
            index_used = Some(IndexType::Hnsw);
            let candidates = u32::try_from(stmt.limit.unwrap_or(50)).unwrap_or(u32::MAX);
            let ef_search = Self::resolve_ef_search(stmt);
            nodes.push(PlanNode::VectorSearch(VectorSearchPlan {
                collection: stmt.from.clone(),
                ef_search,
                candidates,
            }));
        } else if let Some((property, value)) = index_lookup {
            index_used = Some(IndexType::Property);
            nodes.push(PlanNode::IndexLookup(IndexLookupPlan {
                label: stmt.from.clone(),
                property,
                value,
            }));
        } else {
            index_used = None;
            nodes.push(PlanNode::TableScan(TableScanPlan {
                collection: stmt.from.clone(),
            }));
        }

        (nodes, index_used)
    }

    /// Reads `ef_search` from the WITH clause, falling back to [`Self::DEFAULT_EF_SEARCH`].
    #[allow(clippy::cast_possible_truncation)]
    fn resolve_ef_search(stmt: &SelectStatement) -> u32 {
        stmt.with_clause
            .as_ref()
            .and_then(crate::velesql::ast::WithClause::get_ef_search)
            .map_or(Self::DEFAULT_EF_SEARCH, |v| v as u32)
    }

    /// Extracts WITH clause options as display pairs (issue #471).
    fn extract_with_options(stmt: &SelectStatement) -> Vec<(String, String)> {
        let Some(ref wc) = stmt.with_clause else {
            return Vec::new();
        };
        wc.options
            .iter()
            .map(|opt| (opt.key.clone(), formatter::format_with_value(&opt.value)))
            .collect()
    }

    /// Extracts FUSION clause info for EXPLAIN display (issue #471).
    fn extract_fusion_info(stmt: &SelectStatement) -> Option<FusionInfo> {
        let fc = stmt.fusion_clause.as_ref()?;
        let strategy = match fc.strategy {
            crate::velesql::ast::FusionStrategyType::Rrf => "RRF",
            crate::velesql::ast::FusionStrategyType::Weighted => "Weighted",
            crate::velesql::ast::FusionStrategyType::Maximum => "Maximum",
            crate::velesql::ast::FusionStrategyType::Rsf => "RSF",
            crate::velesql::ast::FusionStrategyType::Average => "Average",
        };
        let weights = Self::format_fusion_weights(fc);
        Some(FusionInfo {
            strategy: strategy.to_string(),
            k: fc.k,
            weights,
        })
    }

    /// Formats fusion weights into a human-readable string.
    fn format_fusion_weights(fc: &crate::velesql::ast::FusionClause) -> Option<String> {
        let mut parts = Vec::new();
        if let Some(vw) = fc.vector_weight {
            parts.push(format!("vector={vw}"));
        }
        if let Some(gw) = fc.graph_weight {
            parts.push(format!("graph={gw}"));
        }
        if let Some(dw) = fc.dense_weight {
            parts.push(format!("dense={dw}"));
        }
        if let Some(sw) = fc.sparse_weight {
            parts.push(format!("sparse={sw}"));
        }
        if parts.is_empty() {
            None
        } else {
            Some(parts.join(", "))
        }
    }

    /// Formats LET bindings as `"name = expr"` strings (issue #471).
    fn format_let_bindings(bindings: &[LetBinding]) -> Vec<String> {
        bindings
            .iter()
            .map(|b| format!("{} = {}", b.name, b.expr))
            .collect()
    }

    /// Variant of `append_filter_nodes` with access to the calibrated
    /// `CostEstimator` (issue #471).
    ///
    /// Selectivity and filter strategy use histogram data when `stats` is
    /// `Some`. When `None`, the historical heuristic (selectivity from
    /// condition count, 0.1 threshold) is preserved bit-for-bit.
    fn append_filter_nodes_with_stats(
        nodes: &mut Vec<PlanNode>,
        filter_conditions: &[String],
        stmt: &SelectStatement,
        has_vector_search: bool,
        stats: Option<&CoreCollectionStats>,
    ) -> FilterStrategy {
        let mut filter_strategy = FilterStrategy::None;

        if !filter_conditions.is_empty() {
            let (selectivity, estimation_method, estimated_rows) =
                Self::estimate_filter_stats(stmt, filter_conditions, stats);

            // Reason: plan_builder owns stmt so the real ef_search/candidates
            // are the same values used in `build_scan_node` above
            // (Devin finding 4). These values drive the pre/post-filter cost
            // comparison so it reflects the user's actual WITH clause instead
            // of a fixed k = 10.
            let ef_search = Self::resolve_ef_search(stmt);
            let candidates = u32::try_from(stmt.limit.unwrap_or(50)).unwrap_or(u32::MAX);

            filter_strategy = Self::resolve_filter_strategy(
                selectivity,
                has_vector_search,
                ef_search,
                candidates,
                stats,
            );

            nodes.push(PlanNode::Filter(FilterPlan {
                conditions: filter_conditions.join(" AND "),
                selectivity,
                estimated_rows,
                estimation_method,
            }));
        }

        if let Some(offset) = stmt.offset {
            nodes.push(PlanNode::Offset(OffsetPlan { count: offset }));
        }
        if let Some(limit) = stmt.limit {
            nodes.push(PlanNode::Limit(LimitPlan { count: limit }));
        }

        filter_strategy
    }

    /// Computes `(selectivity, estimation_method, estimated_rows)` for the
    /// filter block. When `stats` is `Some` and the WHERE clause tree is
    /// available, uses `CostEstimator::estimate_condition_selectivity_with_method`
    /// on the non-vector subtree and reports the actual method that produced
    /// the estimate (histogram / cardinality / heuristic — issue #471, Devin
    /// finding 2). Otherwise falls back to the string-count heuristic.
    fn estimate_filter_stats(
        stmt: &SelectStatement,
        filter_conditions: &[String],
        stats: Option<&CoreCollectionStats>,
    ) -> (f64, Option<String>, Option<u64>) {
        match (stats, stmt.where_clause.as_ref()) {
            (Some(s), Some(where_cond)) => {
                let est = CostEstimator::new(s);
                let non_vector = Self::strip_vector_predicates(where_cond);
                let (sel, method) = non_vector.as_ref().map_or(
                    (
                        1.0,
                        crate::velesql::cost_estimator::SelectivityMethod::Heuristic,
                    ),
                    |c| est.estimate_condition_selectivity_with_method(c),
                );
                let total = s.total_points.max(s.row_count);
                // Reason: total*sel is a cardinality estimate; small fractional
                // losses after ceil are acceptable for EXPLAIN display.
                #[allow(
                    clippy::cast_precision_loss,
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss
                )]
                let rows = ((total as f64) * sel).ceil() as u64;
                (sel, Some(method.as_str().to_string()), Some(rows))
            }
            _ => (Self::estimate_selectivity(filter_conditions), None, None),
        }
    }

    /// Returns the where-clause condition tree with `VectorSearch`-family
    /// nodes removed, because selectivity of the non-vector predicates is
    /// what drives the pre/post-filter decision.
    ///
    /// Returns `None` when every branch is vector-related (selectivity = 1.0).
    fn strip_vector_predicates(condition: &Condition) -> Option<Condition> {
        match condition {
            Condition::VectorSearch(_)
            | Condition::VectorFusedSearch(_)
            | Condition::SparseVectorSearch(_)
            | Condition::Similarity(_) => None,
            Condition::And(left, right) => {
                match (
                    Self::strip_vector_predicates(left),
                    Self::strip_vector_predicates(right),
                ) {
                    (Some(l), Some(r)) => Some(Condition::And(Box::new(l), Box::new(r))),
                    (Some(l), None) => Some(l),
                    (None, Some(r)) => Some(r),
                    (None, None) => None,
                }
            }
            Condition::Or(left, right) => {
                match (
                    Self::strip_vector_predicates(left),
                    Self::strip_vector_predicates(right),
                ) {
                    (Some(l), Some(r)) => Some(Condition::Or(Box::new(l), Box::new(r))),
                    // If one branch is vector-only, the OR is satisfied by
                    // the vector side too → treat as selectivity 1.0.
                    _ => None,
                }
            }
            Condition::Not(inner) => {
                Self::strip_vector_predicates(inner).map(|c| Condition::Not(Box::new(c)))
            }
            Condition::Group(inner) => {
                Self::strip_vector_predicates(inner).map(|c| Condition::Group(Box::new(c)))
            }
            other => Some(other.clone()),
        }
    }

    /// Picks `PreFilter` vs `PostFilter` given a pre-computed selectivity,
    /// the real `ef_search` / `candidates` from the query, and optional
    /// calibrated stats.
    ///
    /// When `stats` is `Some` and there's a vector search in the plan, runs
    /// a cost comparison: pre-filter cost (scan + filter then HNSW on the
    /// reduced set) vs post-filter cost (HNSW then filter on k results).
    /// The HNSW cost uses `estimate_hnsw_search_cost_with_ef(ef_search,
    /// candidates)` so the comparison reflects the user's actual WITH clause
    /// (issue #471, Devin finding 4) instead of a hard-coded `k = 10`.
    /// A recall guardrail forces `PostFilter` when selectivity is too loose
    /// (>=0.5) so the pre-scan does not starve HNSW of good candidates.
    ///
    /// When `stats` is `None`, preserves the historical 0.1 threshold.
    fn resolve_filter_strategy(
        selectivity: f64,
        has_vector_search: bool,
        ef_search: u32,
        candidates: u32,
        stats: Option<&CoreCollectionStats>,
    ) -> FilterStrategy {
        let Some(s) = stats else {
            return if selectivity > FALLBACK_SELECTIVITY_THRESHOLD {
                FilterStrategy::PostFilter
            } else {
                FilterStrategy::PreFilter
            };
        };

        if !has_vector_search {
            // No vector search → the notion of pre/post-filter doesn't apply
            // the same way; preserve the historical threshold for parity with
            // the non-vector plan shape.
            return if selectivity > FALLBACK_SELECTIVITY_THRESHOLD {
                FilterStrategy::PostFilter
            } else {
                FilterStrategy::PreFilter
            };
        }

        // Recall guardrail: a loose filter leaves too many candidates in the
        // HNSW frontier, so it's safer to post-filter after HNSW has done
        // the heavy lifting on the full set.
        if selectivity >= PREFILTER_RECALL_GUARD {
            return FilterStrategy::PostFilter;
        }

        let est = CostEstimator::new(s);
        let hnsw_cost = est
            .estimate_hnsw_search_cost_with_ef(ef_search, candidates)
            .total();
        // Pre-filter: full scan + filter evaluation, then HNSW on the
        // reduced set.
        let pre_filter = est
            .estimate_filter_cost_from_selectivity(selectivity)
            .total()
            + hnsw_cost * selectivity;
        // Post-filter: full HNSW pass, then filter evaluation on the
        // top-k results (small constant cost, approximated by selectivity
        // weight applied to the filter cost).
        let post_filter = hnsw_cost
            + est
                .estimate_filter_cost_from_selectivity(selectivity)
                .total()
                * 0.01;

        if pre_filter < post_filter {
            FilterStrategy::PreFilter
        } else {
            FilterStrategy::PostFilter
        }
    }

    /// Analyzes a condition to extract vector search and filter info.
    #[allow(clippy::too_many_lines)]
    fn analyze_condition(
        condition: &Condition,
        has_vector_search: &mut bool,
        filter_conditions: &mut Vec<String>,
    ) {
        match condition {
            Condition::VectorSearch(_)
            | Condition::VectorFusedSearch(_)
            | Condition::SparseVectorSearch(_)
            | Condition::Similarity(_) => {
                *has_vector_search = true;
            }
            Condition::Comparison(cmp) => {
                filter_conditions.push(format!("{} {} ?", cmp.column, cmp.operator.as_str()));
            }
            Condition::In(inc) => {
                let op = if inc.negated { "NOT IN" } else { "IN" };
                filter_conditions.push(format!("{} {op} (...)", inc.column));
            }
            Condition::Between(btw) => {
                filter_conditions.push(format!("{} BETWEEN ? AND ?", btw.column));
            }
            Condition::Like(lk) => {
                filter_conditions.push(format!("{} LIKE ?", lk.column));
            }
            Condition::IsNull(isn) => {
                let op = if isn.is_null {
                    "IS NULL"
                } else {
                    "IS NOT NULL"
                };
                filter_conditions.push(format!("{} {op}", isn.column));
            }
            Condition::Match(m) => {
                filter_conditions.push(format!("{} MATCH ?", m.column));
            }
            Condition::ContainsText(ct) => {
                filter_conditions.push(format!("{} CONTAINS_TEXT ?", ct.column));
            }
            Condition::GraphMatch(_) => {
                filter_conditions.push("MATCH (...)".to_string());
            }
            Condition::Contains(cc) => {
                let mode_str = match cc.mode {
                    crate::velesql::ContainsMode::Single => "CONTAINS",
                    crate::velesql::ContainsMode::Any => "CONTAINS ANY",
                    crate::velesql::ContainsMode::All => "CONTAINS ALL",
                };
                filter_conditions.push(format!("{} {mode_str} ?", cc.column));
            }
            Condition::GeoDistance(gd) => {
                filter_conditions.push(format!(
                    "GEO_DISTANCE({}, {}, {}) {} ?",
                    gd.column,
                    gd.lat,
                    gd.lng,
                    gd.operator.as_str()
                ));
            }
            Condition::GeoBbox(gb) => {
                filter_conditions.push(format!("GEO_BBOX({}, ...)", gb.column));
            }
            Condition::And(left, right) | Condition::Or(left, right) => {
                Self::analyze_condition(left, has_vector_search, filter_conditions);
                Self::analyze_condition(right, has_vector_search, filter_conditions);
            }
            Condition::Not(inner) | Condition::Group(inner) => {
                Self::analyze_condition(inner, has_vector_search, filter_conditions);
            }
        }
    }

    fn extract_index_lookup(
        condition: &Condition,
        indexed_fields: &HashSet<String>,
    ) -> Option<(String, String)> {
        if let Condition::Comparison(cmp) = condition {
            if cmp.operator == crate::velesql::CompareOp::Eq && indexed_fields.contains(&cmp.column)
            {
                return Some((cmp.column.clone(), format!("{:?}", cmp.value)));
            }
        }
        if let Condition::In(inc) = condition {
            if indexed_fields.contains(&inc.column) {
                let op = if inc.negated { "NOT IN" } else { "IN" };
                return Some((inc.column.clone(), format!("{op} (...)")));
            }
        }
        None
    }

    /// Estimates selectivity (placeholder - would need statistics in production).
    pub(crate) fn estimate_selectivity(conditions: &[String]) -> f64 {
        node_stats::estimate_selectivity(conditions, None)
    }

    /// Returns the heuristic cost for a single plan node.
    #[cfg(test)]
    pub(crate) fn node_cost(node: &PlanNode) -> f64 {
        node_stats::node_cost(node)
    }

    /// Extracts traversal parameters from a `MatchExecutionStrategy`.
    fn extract_strategy_info(
        strategy: &MatchExecutionStrategy,
    ) -> (Vec<String>, u32, bool, Option<f32>) {
        match strategy {
            MatchExecutionStrategy::GraphFirst {
                start_labels,
                max_depth,
            } => (start_labels.clone(), *max_depth, false, None),
            MatchExecutionStrategy::VectorFirst { threshold, .. } => {
                (Vec::new(), 1, true, Some(*threshold))
            }
            MatchExecutionStrategy::Parallel {
                graph_hint,
                vector_hint,
            } => {
                let (labels, depth) = match graph_hint.as_ref() {
                    MatchExecutionStrategy::GraphFirst {
                        start_labels,
                        max_depth,
                    } => (start_labels.clone(), *max_depth),
                    _ => (Vec::new(), 1),
                };
                let threshold = match vector_hint.as_ref() {
                    MatchExecutionStrategy::VectorFirst { threshold, .. } => Some(*threshold),
                    _ => None,
                };
                (labels, depth, true, threshold)
            }
        }
    }
}
