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
        let result = self
            .query
            .query_planner
            .choose_strategy_with_cbo_and_overfetch(&col_stats, effective_filter, limit);
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

    /// Returns `true` when the primary ORDER BY key is a scalar column (not a
    /// `similarity()` reduction), so the candidate fetch must be exhaustive.
    ///
    /// The similarity-ordered fast path (HNSW returns top-k pre-sorted by
    /// score) is correct under a bounded fetch, but a scalar key is only
    /// ranked downstream in `apply_select_postprocessing`; truncating the
    /// fetch first would yield the first `limit` rows in storage/score order
    /// rather than the top `limit` by the ORDER BY key. The *primary* key
    /// decides: a leading `similarity()` keeps the fast path even when a
    /// scalar tie-breaker follows it.
    pub(super) fn order_by_requires_exhaustive_fetch(
        stmt: &crate::velesql::SelectStatement,
    ) -> bool {
        stmt.order_by
            .as_ref()
            .and_then(|ob| ob.first())
            .is_some_and(|first| !Self::order_by_item_reduces_to_similarity(&first.expr))
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
        let plan = self.query.query_planner.choose_hybrid_strategy(
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
        let execution_limit = main_select_execution_limit(stmt, extracted, limit);
        let search_opts = super::QuerySearchOptions::from_with_clause(stmt.with_clause.as_ref())
            .with_fusion(stmt.fusion_clause.clone());
        let (cbo_strategy, cbo_over_fetch) =
            self.compute_cbo_strategy(stmt, extracted.filter_condition.as_ref(), limit);

        // GraphFirst by anchor ids: AND-required MATCH predicates are
        // evaluated FIRST so retrieval is exhaustive within the graph
        // matches instead of bounded by an over-fetch window. The cache
        // carries the computed anchor sets into the exact post-filter.
        //
        // When ORDER BY similarity() is present without a NEAR vector,
        // fetch_anchor_candidates must see EVERY anchor — the similarity
        // sort only runs downstream, so a bounded window would drop the
        // most-similar anchors by ascending-id order (see anchor_fetch_limit).
        let mut graph_cache = super::where_eval::GraphMatchEvalCache::default();
        let anchor_fetch_limit = anchor_fetch_limit(stmt, extracted, limit);
        let anchored = self.try_anchored_fetch(
            stmt,
            params,
            extracted,
            anchor_fetch_limit,
            &mut graph_cache,
        )?;

        let mut results = self.resolve_initial_results(
            anchored,
            stmt,
            params,
            extracted,
            execution_limit,
            skip_metadata_prefilter_for_graph_or,
            &search_opts,
            cbo_strategy,
            cbo_over_fetch,
            &mut graph_cache,
        )?;

        if has_graph_predicates {
            results = self.post_filter_graph_where(stmt, params, results, &mut graph_cache)?;
        }

        Ok(results)
    }

    /// Resolves the initial result set from the anchored, hybrid-anchored,
    /// or fallback vector-query path.
    #[allow(clippy::too_many_arguments)]
    fn resolve_initial_results(
        &self,
        anchored: Option<Vec<SearchResult>>,
        stmt: &crate::velesql::SelectStatement,
        params: &std::collections::HashMap<String, serde_json::Value>,
        extracted: &ExtractedComponents,
        execution_limit: usize,
        skip_metadata_prefilter_for_graph_or: bool,
        search_opts: &super::QuerySearchOptions,
        cbo_strategy: crate::velesql::ExecutionStrategy,
        cbo_over_fetch: usize,
        graph_cache: &mut super::where_eval::GraphMatchEvalCache,
    ) -> Result<Vec<SearchResult>> {
        if let Some(results) = anchored {
            return Ok(results);
        }
        // Anchored hybrid: graph MATCH + text MATCH + NEAR vector.
        // The standard anchored path skips text-MATCH shapes; handle
        // them here by restricting hybrid fusion to the anchor set so
        // relevant anchors outside the global top-K are not missed.
        let hybrid_anchored =
            self.try_anchored_hybrid_fetch(stmt, params, extracted, execution_limit, graph_cache)?;
        if let Some(r) = hybrid_anchored {
            return Ok(r);
        }
        self.dispatch_vector_query(
            extracted.vector_search.as_ref(),
            extracted.similarity_conditions.first(),
            &extracted.similarity_conditions,
            extracted.filter_condition.as_ref(),
            execution_limit,
            skip_metadata_prefilter_for_graph_or,
            search_opts,
            cbo_strategy,
            cbo_over_fetch,
        )
        .inspect_err(|_| self.runtime.guard_rails.circuit_breaker.record_failure())
    }

    /// Applies the exact WHERE post-filter when graph predicates are present,
    /// reusing the warmed anchor cache.
    fn post_filter_graph_where(
        &self,
        stmt: &crate::velesql::SelectStatement,
        params: &std::collections::HashMap<String, serde_json::Value>,
        results: Vec<SearchResult>,
        graph_cache: &mut super::where_eval::GraphMatchEvalCache,
    ) -> Result<Vec<SearchResult>> {
        let Some(cond) = stmt.where_clause.as_ref() else {
            return Ok(results);
        };
        self.apply_where_condition_to_results_with_cache(
            results,
            cond,
            params,
            &stmt.from_alias,
            graph_cache,
        )
        .inspect_err(|_| self.runtime.guard_rails.circuit_breaker.record_failure())
    }

    /// Attempts the GraphFirst anchored fetch for the main SELECT path.
    ///
    /// Applicable when the WHERE clause AND-requires at least one MATCH
    /// predicate and the fetch is either plain NEAR or unranked — the
    /// similarity()-cascade and BM25 text-fusion shapes keep their dedicated
    /// scoring pipelines (the post-filter window applies there).
    ///
    /// Returns `Ok(None)` to fall through to the regular dispatch.
    fn try_anchored_fetch(
        &self,
        stmt: &crate::velesql::SelectStatement,
        params: &std::collections::HashMap<String, serde_json::Value>,
        extracted: &ExtractedComponents,
        limit: usize,
        graph_cache: &mut super::where_eval::GraphMatchEvalCache,
    ) -> Result<Option<Vec<SearchResult>>> {
        let Some(cond) = stmt.where_clause.as_ref() else {
            return Ok(None);
        };
        if !anchored_fetch_applies(extracted, cond) {
            return Ok(None);
        }
        let Some(anchor_ids) =
            self.compute_required_anchor_ids(cond, params, &stmt.from_alias, graph_cache)?
        else {
            return Ok(None);
        };
        let results = match extracted.vector_search.as_ref() {
            Some(vector) => self.search_near_with_anchor_ids(
                vector,
                &anchor_ids,
                extracted.filter_condition.as_ref(),
                limit,
            )?,
            None => self.fetch_anchor_candidates(
                &anchor_ids,
                cond,
                params,
                &stmt.from_alias,
                graph_cache,
                limit,
            )?,
        };
        Ok(Some(results))
    }

    /// Anchored hybrid fetch for the `(graph MATCH ∧ text MATCH ∧ NEAR)` shape.
    ///
    /// The standard `try_anchored_fetch` skips this shape (text MATCH shapes
    /// keep their own fusion pipeline). This path bridges the gap: it computes
    /// graph anchor IDs and runs hybrid RRF fusion restricted to those IDs,
    /// so relevant anchors outside the global top-K are not silently missed.
    ///
    /// Returns `Ok(None)` when the shape does not apply or anchors are
    /// unavailable (falls through to the regular dispatch).
    fn try_anchored_hybrid_fetch(
        &self,
        stmt: &crate::velesql::SelectStatement,
        params: &std::collections::HashMap<String, serde_json::Value>,
        extracted: &ExtractedComponents,
        limit: usize,
        graph_cache: &mut super::where_eval::GraphMatchEvalCache,
    ) -> Result<Option<Vec<SearchResult>>> {
        let (Some(vector), Some(cond)) =
            (extracted.vector_search.as_ref(), stmt.where_clause.as_ref())
        else {
            return Ok(None);
        };
        if extracted.graph_match_predicates.is_empty() {
            return Ok(None);
        }
        let Some(text_query) = Self::extract_match_query(cond) else {
            return Ok(None);
        };
        // Only AND-required graph predicates justify anchor restriction.
        if !Self::graph_predicates_are_and_required(cond) {
            return Ok(None);
        }
        let Some(anchor_ids) =
            self.compute_required_anchor_ids(cond, params, &stmt.from_alias, graph_cache)?
        else {
            return Ok(None);
        };

        // Route through the strategy-honoring helper so maximum/average/rsf/
        // weighted are applied to the anchor-restricted streams (#6, anchored).
        // The anchor set constrains both branches identically, so only the
        // fusion of the two score streams changes; rrf/unset stays byte-identical.
        let results = self.hybrid_search_with_anchors_clause(
            vector,
            &text_query,
            limit,
            stmt.fusion_clause.as_ref(),
            &anchor_ids,
        )?;
        Ok(Some(results))
    }

    /// Returns `true` when the condition tree has no top-level OR wrapping the
    /// graph MATCH predicates — i.e., graph predicates are AND-required and
    /// anchor restriction is exhaustive.
    fn graph_predicates_are_and_required(cond: &crate::velesql::Condition) -> bool {
        !Self::condition_contains_or(cond)
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

    /// Applies DISTINCT, window functions, ORDER BY (with LET bindings), OFFSET, LIMIT, and
    /// LET payload injection (Issue #473).
    ///
    /// # Pipeline order and its SQL-standard deviation
    ///
    /// VelesQL runs `DISTINCT → window functions → ORDER BY → OFFSET/LIMIT`.
    /// Standard SQL runs window functions **before** DISTINCT (logical order
    /// `SELECT → DISTINCT → ORDER BY`). This is an **intentional deviation**
    /// tailored to the vector-search use case:
    ///
    /// - "Give me the top-N distinct titles, ranked by similarity" (the
    ///   common vector-search pattern) wants DISTINCT to collapse rows
    ///   **before** ROW_NUMBER / RANK assigns positions, so survivors get
    ///   a dense `1..N` numbering. Running window functions first would
    ///   leave gaps in the numbering after DISTINCT drops rows.
    /// - No VelesQL query currently uses the standard-SQL contract, so no
    ///   existing user code depends on the reverse order.
    ///
    /// If the standard order becomes necessary in the future (e.g., SQL
    /// compatibility mode), swap step 1 and step 2 and wrap behind a feature
    /// flag. Regression coverage is in
    /// `window_function_tests::test_distinct_runs_before_window_functions`.
    pub(super) fn apply_select_postprocessing(
        &self,
        stmt: &crate::velesql::SelectStatement,
        mut results: Vec<SearchResult>,
        params: &std::collections::HashMap<String, serde_json::Value>,
        limit: usize,
        let_bindings: &[crate::velesql::LetBinding],
    ) -> Result<Vec<SearchResult>> {
        // Step 1: DISTINCT — deduplication before any ranking (see pipeline
        // order contract in the doc comment above).
        if stmt.distinct == crate::velesql::DistinctMode::All {
            results = distinct::apply_distinct(results, &stmt.columns);
        }
        // Step 2: Window functions — after DISTINCT, before ORDER BY/LIMIT.
        if let Some(wfs) = Self::extract_window_functions(&stmt.columns) {
            crate::velesql::window_evaluator::evaluate(&mut results, wfs)?;
        }
        // Step 3: ORDER BY (with optional LET bindings).
        self.apply_order_by_step(stmt, &mut results, params, let_bindings)?;
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

    /// Apply ORDER BY, choosing the plain or LET-aware path depending on
    /// whether any LET bindings are in scope.
    fn apply_order_by_step(
        &self,
        stmt: &crate::velesql::SelectStatement,
        results: &mut [SearchResult],
        params: &std::collections::HashMap<String, serde_json::Value>,
        let_bindings: &[crate::velesql::LetBinding],
    ) -> Result<()> {
        let Some(ref order_by) = stmt.order_by else {
            return Ok(());
        };
        if let_bindings.is_empty() {
            self.apply_order_by(results, order_by, params)?;
        } else {
            let per_result_let = Self::evaluate_let_for_results(let_bindings, results);
            self.apply_order_by_with_let(results, order_by, params, &per_result_let)?;
        }
        Ok(())
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

    /// Extracts window functions from `SelectColumns`, if any are present.
    fn extract_window_functions(
        columns: &crate::velesql::SelectColumns,
    ) -> Option<&[crate::velesql::WindowFunction]> {
        match columns {
            crate::velesql::SelectColumns::Mixed {
                window_functions, ..
            } if !window_functions.is_empty() => Some(window_functions),
            _ => None,
        }
    }
}

/// Bounded over-fetch for SELECTs combining graph MATCH predicates with a
/// RANKED fetch (`vector NEAR` or a `similarity()` threshold).
///
/// Graph predicates are evaluated AFTER the vector fetch
/// (`apply_where_condition_to_results`), so the fetch over-samples to leave
/// the graph filter enough surviving candidates. The previous blanket
/// `MAX_LIMIT` (100k) hydrated up to 100k points per query and drove the
/// downstream oversampling clamp into a `min > max` panic for filtered
/// vector searches. Bound it to 10x the requested limit, capped at 10_000
/// candidates (never below the user limit). Trade-off: graph-matching rows
/// ranked beyond the over-fetch window are not surfaced; exhaustive
/// retrieval should pre-filter by graph anchor ids instead.
///
/// Unranked metadata/scan fetches must NOT use this bound: they iterate in
/// storage order, so a capped window would silently drop graph matches based
/// on insertion order (those paths keep `MAX_LIMIT`, the pre-existing
/// behavior).
/// Whether the GraphFirst anchored fetch covers this query shape: graph
/// predicates present, and no similarity() cascade or BM25 text-MATCH
/// fusion (those keep their dedicated scoring pipelines).
fn anchored_fetch_applies(
    extracted: &ExtractedComponents,
    cond: &crate::velesql::Condition,
) -> bool {
    !extracted.graph_match_predicates.is_empty()
        && extracted.similarity_conditions.is_empty()
        && Collection::extract_match_query(cond).is_none()
}

/// Fetch window for the main SELECT path.
///
/// The bounded over-fetch window only makes sense when the fetch is RANKED
/// (vector NEAR or similarity() threshold): "rows ranked beyond the window"
/// is a meaningful trade-off there. The metadata/scan paths fetch in storage
/// order — capping them would silently drop graph matches depending on
/// insertion order, so they keep the exhaustive MAX_LIMIT window (sparse
/// queries never reach here; they are dispatched by `try_early_return_path`).
/// This window only applies when the GraphFirst anchored fetch declined
/// (`try_anchored_fetch`).
fn main_select_execution_limit(
    stmt: &crate::velesql::SelectStatement,
    extracted: &ExtractedComponents,
    limit: usize,
) -> usize {
    // A scalar (non-similarity) ORDER BY ranks rows AFTER the fetch, in
    // `apply_select_postprocessing`. Capping the fetch at `limit` would
    // truncate before that sort, returning the first `limit` rows in
    // storage/score order instead of the top `limit` by the ORDER BY key
    // (KNOWN_LIMITATIONS #9: bounded results must equal the unbounded path
    // truncated to k). Fetch exhaustively so the sort precedes truncation.
    if Collection::order_by_requires_exhaustive_fetch(stmt) {
        return MAX_LIMIT;
    }
    let has_graph_predicates = !extracted.graph_match_predicates.is_empty();
    let has_ranked_fetch =
        extracted.vector_search.is_some() || !extracted.similarity_conditions.is_empty();
    match (has_graph_predicates, has_ranked_fetch) {
        (true, true) => graph_overfetch_limit(limit),
        (true, false) => MAX_LIMIT,
        (false, _) => limit,
    }
}

fn graph_overfetch_limit(limit: usize) -> usize {
    const GRAPH_OVERFETCH_CAP: usize = 10_000;
    limit.max(limit.saturating_mul(10).min(GRAPH_OVERFETCH_CAP))
}

/// Anchored-fetch window for the main SELECT path.
///
/// With ORDER BY similarity() and no NEAR vector the anchored fetch must be
/// EXHAUSTIVE: `fetch_anchor_candidates` hydrates anchors in ascending-id
/// order and stops at the window, while the similarity sort only runs
/// downstream in `apply_order_by` — any bounded window therefore drops the
/// most-similar anchors whenever the anchor set is larger than the window.
/// `MAX_LIMIT` is the same exhaustive window the unranked graph paths use.
/// Without ORDER BY similarity() the plain `limit` stays (nothing ranked to
/// protect); with a NEAR vector the anchored search ranks inside the anchor
/// set already.
fn anchor_fetch_limit(
    stmt: &crate::velesql::SelectStatement,
    extracted: &ExtractedComponents,
    limit: usize,
) -> usize {
    // A scalar ORDER BY ranks downstream, so the anchored fetch (which
    // hydrates anchors in ascending-id order) must be exhaustive too —
    // otherwise the ascending-id window drops rows the ORDER BY key would
    // have surfaced. Mirrors `main_select_execution_limit`.
    if Collection::order_by_requires_exhaustive_fetch(stmt) {
        return MAX_LIMIT;
    }
    if Collection::has_order_by_similarity(stmt) && extracted.vector_search.is_none() {
        MAX_LIMIT
    } else {
        limit
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
