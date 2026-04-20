//! Internal dispatch helpers for SELECT query execution.
//!
//! Extracted from the main `query/mod.rs` to keep that file under 500 NLOC.
//! These methods handle CBO strategy, main SELECT dispatch, JOIN pushdown
//! analysis, and post-processing (DISTINCT / ORDER BY / LIMIT).
//!
//! MATCH-specific dispatch lives in `match_dispatch.rs` (Extract Module).

use crate::collection::types::Collection;
use crate::error::Result;
use crate::point::SearchResult;

use super::{distinct, pushdown, ExtractedComponents, MAX_LIMIT};

impl Collection {
    /// Computes the CBO execution strategy and over-fetch factor for the query.
    ///
    /// Routes between two planner entry points depending on query shape
    /// (issue #467 closure):
    ///
    /// - **Queries with `ORDER BY similarity()`** must preserve HNSW's natural
    ///   similarity ordering, so the planner forces `VectorFirst` via
    ///   [`QueryPlanner::choose_hybrid_strategy`], which additionally computes
    ///   a selectivity-aware over-fetch factor clamped to `[2.0, 10.0]` when
    ///   a filter is present. A pure-cost comparison would occasionally pick
    ///   `GraphFirst`/`Parallel` and lose the natural ordering — observable
    ///   as scrambled top-k for `ORDER BY similarity() DESC LIMIT k` queries.
    /// - **All other queries** use the calibrated
    ///   [`QueryPlanner::choose_strategy_with_cbo_and_overfetch`] path, which
    ///   derives I/O / CPU weights from `OperationCostFactors` (or defaults
    ///   when the collection was never analyzed).
    pub(super) fn compute_cbo_strategy(
        &self,
        stmt: &crate::velesql::SelectStatement,
        filter_condition: Option<&crate::velesql::Condition>,
        limit: usize,
    ) -> (crate::velesql::ExecutionStrategy, usize) {
        // `filter_condition` is the WHERE clause AFTER `extract_vector_search`
        // and similarity extraction — for pure `vector NEAR $v` queries the
        // residual is a vector-only node that carries no selectivity signal.
        // Strip the vector-family subtree so the CBO does not compute a
        // spurious over-fetch (Devin PR #613 finding 3).
        let meaningful_filter = filter_condition.and_then(crate::velesql::strip_vector_predicates);
        let effective_filter = meaningful_filter.as_ref();

        if Self::has_order_by_similarity(stmt) {
            return self.cbo_strategy_for_order_by_similarity(effective_filter, limit);
        }
        let col_stats = self.get_stats();
        let result = self.query_planner.choose_strategy_with_cbo_and_overfetch(
            &col_stats,
            effective_filter,
            limit,
        );
        tracing::debug!(
            strategy = ?result.0, over_fetch = result.1,
            "CBO selected execution strategy (calibrated cost path)"
        );
        result
    }

    /// Returns `true` when the ORDER BY clause contains at least one
    /// expression whose final ordering reduces to `similarity()` under a
    /// monotonic transform. This routes the query through
    /// `choose_hybrid_strategy` so HNSW's natural similarity ordering is
    /// preserved by the executor.
    ///
    /// Detected shapes:
    /// - Top-level `OrderByExpr::Similarity(_)` / `OrderByExpr::SimilarityBare`.
    /// - `OrderByExpr::Arithmetic(...)` whose expression tree contains a
    ///   `Similarity` node and **no other `Variable` reference** — i.e.
    ///   `similarity() * 2.0`, `0.5 * similarity() + 0.25`,
    ///   `-similarity() + 1.0` are all monotonic (Devin PR #613 finding 1).
    ///
    /// Composite expressions such as `0.7 * similarity() + 0.3 * bm25_score`
    /// carry a `Variable` node and are deliberately NOT detected — their
    /// final ordering differs from pure similarity, so forcing VectorFirst
    /// would trade correctness for an inconsequential optimisation.
    fn has_order_by_similarity(stmt: &crate::velesql::SelectStatement) -> bool {
        let Some(order_by) = stmt.order_by.as_ref() else {
            return false;
        };
        order_by
            .iter()
            .any(|item| Self::order_by_item_reduces_to_similarity(&item.expr))
    }

    /// Helper for [`has_order_by_similarity`]. Kept as an associated function
    /// so the match arm can delegate to the arithmetic-expression walker
    /// without inflating the outer method's cyclomatic complexity.
    fn order_by_item_reduces_to_similarity(expr: &crate::velesql::OrderByExpr) -> bool {
        use crate::velesql::OrderByExpr;
        match expr {
            OrderByExpr::Similarity(_) | OrderByExpr::SimilarityBare => true,
            OrderByExpr::Arithmetic(arith) => {
                Self::arith_contains_similarity(arith) && !Self::arith_contains_variable(arith)
            }
            _ => false,
        }
    }

    /// Returns `true` if any node in the arithmetic expression tree is a
    /// `Similarity` call.
    fn arith_contains_similarity(expr: &crate::velesql::ArithmeticExpr) -> bool {
        use crate::velesql::ArithmeticExpr;
        match expr {
            ArithmeticExpr::Similarity(_) => true,
            ArithmeticExpr::BinaryOp { left, right, .. } => {
                Self::arith_contains_similarity(left) || Self::arith_contains_similarity(right)
            }
            _ => false,
        }
    }

    /// Returns `true` if any node in the arithmetic expression tree is a
    /// `Variable` reference — used to reject composite scoring like
    /// `similarity() + bm25_score`.
    fn arith_contains_variable(expr: &crate::velesql::ArithmeticExpr) -> bool {
        use crate::velesql::ArithmeticExpr;
        match expr {
            ArithmeticExpr::Variable(_) => true,
            ArithmeticExpr::BinaryOp { left, right, .. } => {
                Self::arith_contains_variable(left) || Self::arith_contains_variable(right)
            }
            _ => false,
        }
    }

    /// CBO path for queries that carry `ORDER BY similarity()` in their
    /// projection. Delegates to [`QueryPlanner::choose_hybrid_strategy`] so
    /// the returned `HybridExecutionPlan.strategy` is always `VectorFirst`
    /// and the over-fetch factor reflects the calibrated selectivity.
    fn cbo_strategy_for_order_by_similarity(
        &self,
        filter_condition: Option<&crate::velesql::Condition>,
        limit: usize,
    ) -> (crate::velesql::ExecutionStrategy, usize) {
        let col_stats = self.get_stats();
        let estimated_selectivity = filter_condition.map(|cond| {
            crate::velesql::CostEstimator::new(&col_stats)
                .estimate_condition_selectivity(cond)
                .clamp(0.001, 1.0)
        });
        // Reason: execution limit already clamped upstream to `usize::MAX`
        // equivalent values; `u64::try_from(usize)` never fails on 64-bit
        // targets and saturates on 32-bit.
        let limit_u64 = u64::try_from(limit).unwrap_or(u64::MAX);
        let plan = self.query_planner.choose_hybrid_strategy(
            true, // has_order_by_similarity
            filter_condition.is_some(),
            Some(limit_u64),
            estimated_selectivity,
        );
        // Reason: `choose_hybrid_strategy` with `has_order_by_similarity = true`
        // produces `over_fetch_factor` in `[1.0, 10.0]` — either `1.0` when no
        // filter is present, or `(1.0 / selectivity).clamp(2.0, 10.0)` with a
        // filter (planner.rs:265-275). Ceil-to-usize is therefore safe and
        // lossless. `.max(1)` guards against a degenerate planner output
        // (would be a bug, not a truncation).
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss
        )]
        let over_fetch = (plan.over_fetch_factor.ceil() as usize).max(1);
        tracing::debug!(
            strategy = ?plan.strategy,
            over_fetch,
            use_early_termination = plan.use_early_termination,
            recompute_scores = plan.recompute_scores,
            "CBO selected execution strategy (ORDER BY similarity() path)"
        );
        (plan.strategy, over_fetch)
    }

    /// Dispatches the main SELECT query path (vector, similarity, metadata).
    pub(super) fn dispatch_main_select(
        &self,
        stmt: &crate::velesql::SelectStatement,
        params: &std::collections::HashMap<String, serde_json::Value>,
        extracted: &ExtractedComponents,
        limit: usize,
        _ctx: &crate::guardrails::QueryContext,
    ) -> Result<Vec<SearchResult>> {
        let has_graph_predicates = !extracted.graph_match_predicates.is_empty();
        let skip_metadata_prefilter_for_graph_or = has_graph_predicates
            && stmt
                .where_clause
                .as_ref()
                .is_some_and(Self::condition_contains_or);
        let execution_limit = if has_graph_predicates {
            MAX_LIMIT
        } else {
            limit
        };
        let search_opts = super::QuerySearchOptions::from_with_clause(stmt.with_clause.as_ref())
            .with_fusion(stmt.fusion_clause.clone());
        let first_similarity = extracted.similarity_conditions.first().cloned();
        let (cbo_strategy, cbo_over_fetch) =
            self.compute_cbo_strategy(stmt, extracted.filter_condition.as_ref(), limit);

        let mut results = self
            .dispatch_vector_query(
                extracted.vector_search.as_ref(),
                first_similarity.as_ref(),
                &extracted.similarity_conditions,
                extracted.filter_condition.as_ref(),
                execution_limit,
                skip_metadata_prefilter_for_graph_or,
                &search_opts,
                cbo_strategy,
                cbo_over_fetch,
            )
            .inspect_err(|_| self.guard_rails.circuit_breaker.record_failure())?;

        if has_graph_predicates {
            if let Some(cond) = stmt.where_clause.as_ref() {
                results = self
                    .apply_where_condition_to_results(results, cond, params, &stmt.from_alias)
                    .inspect_err(|_| self.guard_rails.circuit_breaker.record_failure())?;
            }
        }

        Ok(results)
    }

    /// Analyzes JOIN pushdown opportunities (EPIC-031 US-006).
    ///
    /// Returns a [`PushdownAnalysis`](pushdown::PushdownAnalysis) classifying
    /// WHERE conditions by data source so the caller can route each filter to
    /// the correct execution stage.
    #[allow(clippy::unused_self)]
    pub(super) fn analyze_join_pushdown(
        &self,
        stmt: &crate::velesql::SelectStatement,
    ) -> pushdown::PushdownAnalysis {
        if stmt.joins.is_empty() {
            return pushdown::PushdownAnalysis::default();
        }
        let Some(ref cond) = stmt.where_clause else {
            return pushdown::PushdownAnalysis::default();
        };
        let graph_vars: std::collections::HashSet<String> =
            stmt.from_alias.iter().cloned().collect();
        let join_tables = pushdown::extract_join_tables(&stmt.joins);
        let analysis = pushdown::analyze_for_pushdown(cond, &graph_vars, &join_tables);
        tracing::debug!(
            column_store_filters = analysis.column_store_filters.len(),
            graph_filters = analysis.graph_filters.len(),
            post_join_filters = analysis.post_join_filters.len(),
            has_pushdown = analysis.has_pushdown(),
            "JOIN pushdown analysis complete"
        );
        analysis
    }

    /// Applies DISTINCT, ORDER BY (with LET bindings), OFFSET, LIMIT, and
    /// LET payload injection (Issue #473).
    pub(super) fn apply_select_postprocessing(
        &self,
        stmt: &crate::velesql::SelectStatement,
        mut results: Vec<SearchResult>,
        params: &std::collections::HashMap<String, serde_json::Value>,
        limit: usize,
        let_bindings: &[crate::velesql::LetBinding],
    ) -> Result<Vec<SearchResult>> {
        if stmt.distinct == crate::velesql::DistinctMode::All {
            results = distinct::apply_distinct(results, &stmt.columns);
        }
        if let Some(ref order_by) = stmt.order_by {
            if let_bindings.is_empty() {
                self.apply_order_by(&mut results, order_by, params)?;
            } else {
                let per_result_let = Self::evaluate_let_for_results(let_bindings, &results);
                self.apply_order_by_with_let(&mut results, order_by, params, &per_result_let)?;
            }
        }
        // SQL-standard: OFFSET applied after ORDER BY, before LIMIT.
        if let Some(offset) = stmt.offset {
            let skip = usize::try_from(offset).unwrap_or(usize::MAX);
            results = results.into_iter().skip(skip).collect();
        }
        results.truncate(limit);

        // Issue #473: Inject LET binding values into result payloads so they
        // appear in SELECT projection and API responses.
        if !let_bindings.is_empty() {
            let per_result_let = Self::evaluate_let_for_results(let_bindings, &results);
            inject_let_into_payloads(&mut results, &per_result_let);
        }

        Ok(results)
    }

    /// Evaluates LET bindings for every result, producing per-result binding maps.
    fn evaluate_let_for_results(
        let_bindings: &[crate::velesql::LetBinding],
        results: &[SearchResult],
    ) -> Vec<Vec<(String, f32)>> {
        results
            .iter()
            .map(|r| {
                super::ordering::evaluate_let_bindings(
                    let_bindings,
                    r.score,
                    r.point.payload.as_ref(),
                    r.component_scores.as_deref(),
                )
            })
            .collect()
    }
}

/// Injects evaluated LET binding values into each result's payload.
///
/// This makes LET bindings visible in SELECT projection and API responses.
/// LET bindings take precedence over payload fields with the same name.
fn inject_let_into_payloads(results: &mut [SearchResult], per_result_let: &[Vec<(String, f32)>]) {
    for (result, bindings) in results.iter_mut().zip(per_result_let.iter()) {
        if bindings.is_empty() {
            continue;
        }
        let payload = result
            .point
            .payload
            .get_or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
        if let serde_json::Value::Object(map) = payload {
            for (name, value) in bindings {
                map.insert(name.clone(), serde_json::Value::from(f64::from(*value)));
            }
        }
    }
}
