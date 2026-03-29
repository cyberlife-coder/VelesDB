//! ORDER BY clause execution for VelesQL queries.
//!
//! Handles multi-column sorting with support for:
//! - Metadata field sorting (ASC/DESC)
//! - similarity() function sorting
//! - Arithmetic expression sorting (EPIC-042)
//! - Mixed type JSON value comparison with total ordering

use crate::collection::types::Collection;
use crate::error::Result;
use crate::point::SearchResult;
use crate::velesql::{ArithmeticExpr, ArithmeticOp};
use std::cmp::Ordering;

/// Compare two JSON values for sorting with total ordering.
///
/// Ordering priority (ascending): Null < Bool < Number < String < Array < Object
/// This ensures deterministic sorting even with mixed types.
#[must_use]
pub fn compare_json_values(
    a: Option<&serde_json::Value>,
    b: Option<&serde_json::Value>,
) -> Ordering {
    match (a, b) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Less,
        (Some(_), None) => Ordering::Greater,
        (Some(va), Some(vb)) => {
            // BUG FIX: Define total ordering for mixed JSON types
            // Type priority: Null(0) < Bool(1) < Number(2) < String(3) < Array(4) < Object(5)
            let type_rank = |v: &serde_json::Value| -> u8 {
                match v {
                    serde_json::Value::Null => 0,
                    serde_json::Value::Bool(_) => 1,
                    serde_json::Value::Number(_) => 2,
                    serde_json::Value::String(_) => 3,
                    serde_json::Value::Array(_) => 4,
                    serde_json::Value::Object(_) => 5,
                }
            };

            let rank_a = type_rank(va);
            let rank_b = type_rank(vb);

            // First compare by type rank
            if rank_a != rank_b {
                return rank_a.cmp(&rank_b);
            }

            // Same type: compare values
            match (va, vb) {
                (serde_json::Value::Number(na), serde_json::Value::Number(nb)) => {
                    let fa = na.as_f64().unwrap_or(0.0);
                    let fb = nb.as_f64().unwrap_or(0.0);
                    fa.total_cmp(&fb) // Use total_cmp for NaN safety
                }
                (serde_json::Value::String(sa), serde_json::Value::String(sb)) => sa.cmp(sb),
                (serde_json::Value::Bool(ba), serde_json::Value::Bool(bb)) => ba.cmp(bb),
                // Null vs Null, Array vs Array, Object vs Object: treat as equal
                // (comparing array/object contents would be complex and rarely needed)
                _ => Ordering::Equal,
            }
        }
    }
}

impl Collection {
    /// Apply ORDER BY clause to results.
    ///
    /// Supports multiple ORDER BY columns with stable sorting.
    /// Each column is compared in order; ties are broken by subsequent columns.
    ///
    /// # Examples
    ///
    /// ```sql
    /// SELECT * FROM collection ORDER BY category ASC, priority DESC
    /// SELECT * FROM collection ORDER BY similarity() DESC, timestamp ASC
    /// ```
    pub(crate) fn apply_order_by(
        &self,
        results: &mut [SearchResult],
        order_by: &[crate::velesql::SelectOrderBy],
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<()> {
        self.apply_order_by_with_let(results, order_by, params, &[])
    }

    /// Apply ORDER BY with pre-evaluated LET bindings (VelesQL v1.10 Phase 3).
    ///
    /// `let_bindings` contains per-result pre-evaluated `(name, value)` pairs
    /// stored as `[result_idx][(name, value)]`. If empty, behaves identically
    /// to [`apply_order_by`].
    pub(crate) fn apply_order_by_with_let(
        &self,
        results: &mut [SearchResult],
        order_by: &[crate::velesql::SelectOrderBy],
        params: &std::collections::HashMap<String, serde_json::Value>,
        per_result_let: &[Vec<(String, f32)>],
    ) -> Result<()> {
        if order_by.is_empty() {
            return Ok(());
        }

        let similarity_scores_map = self.precompute_similarity_scores(results, order_by, params)?;
        let higher_is_better = self.config.read().metric.higher_is_better();

        let mut indices: Vec<usize> = (0..results.len()).collect();
        indices.sort_by(|&i, &j| {
            Self::compare_by_order_columns(
                i,
                j,
                results,
                order_by,
                &similarity_scores_map,
                higher_is_better,
                per_result_let,
            )
        });

        let sorted_results: Vec<SearchResult> =
            indices.iter().map(|&i| results[i].clone()).collect();
        results.clone_from_slice(&sorted_results);

        // Write back the score from the first similarity column (any position).
        let first_sim_idx = order_by
            .iter()
            .enumerate()
            .find(|(_, ob)| {
                matches!(
                    ob.expr,
                    crate::velesql::OrderByExpr::Similarity(_)
                        | crate::velesql::OrderByExpr::SimilarityBare
                )
            })
            .map(|(idx, _)| idx);
        if let Some(sim_idx) = first_sim_idx {
            if let Some(scores) = similarity_scores_map.get(&sim_idx) {
                for (i, result) in results.iter_mut().enumerate() {
                    result.score = scores[indices[i]];
                }
            }
        }

        Ok(())
    }

    /// Pre-computes similarity scores for all ORDER BY similarity() columns.
    fn precompute_similarity_scores(
        &self,
        results: &[SearchResult],
        order_by: &[crate::velesql::SelectOrderBy],
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<std::collections::HashMap<usize, Vec<f32>>> {
        use crate::velesql::OrderByExpr;
        let mut map = std::collections::HashMap::new();
        for (idx, ob) in order_by.iter().enumerate() {
            match &ob.expr {
                OrderByExpr::Similarity(sim) => {
                    let order_vec = Self::resolve_vector(&sim.vector, params)?;
                    let scores: Vec<f32> = results
                        .iter()
                        .map(|r| self.compute_metric_score(&r.point.vector, &order_vec))
                        .collect();
                    map.insert(idx, scores);
                }
                OrderByExpr::SimilarityBare => {
                    // Zero-arg similarity(): use existing search scores (no recompute).
                    let scores: Vec<f32> = results.iter().map(|r| r.score).collect();
                    map.insert(idx, scores);
                }
                OrderByExpr::Field(_) | OrderByExpr::Aggregate(_) | OrderByExpr::Arithmetic(_) => {}
            }
        }
        Ok(map)
    }

    /// Compares two result indices across all ORDER BY columns.
    #[allow(clippy::too_many_arguments)]
    fn compare_by_order_columns(
        i: usize,
        j: usize,
        results: &[SearchResult],
        order_by: &[crate::velesql::SelectOrderBy],
        similarity_scores: &std::collections::HashMap<usize, Vec<f32>>,
        higher_is_better: bool,
        per_result_let: &[Vec<(String, f32)>],
    ) -> Ordering {
        use crate::velesql::OrderByExpr;
        for (idx, ob) in order_by.iter().enumerate() {
            let cmp = match &ob.expr {
                OrderByExpr::Similarity(_) | OrderByExpr::SimilarityBare => similarity_scores
                    .get(&idx)
                    .map_or(Ordering::Equal, |scores| scores[i].total_cmp(&scores[j])),
                OrderByExpr::Field(field_name) => {
                    Self::compare_field_or_let(field_name, i, j, results, per_result_let)
                }
                OrderByExpr::Aggregate(_) => Ordering::Equal,
                // Design: Arithmetic ORDER BY uses direct numeric ordering without
                // distance-metric inversion. Users constructing custom formulas with
                // Euclidean/Hamming scores should account for lower-is-better semantics
                // in their expression (e.g., `ORDER BY -1 * vector_score + price ASC`).
                OrderByExpr::Arithmetic(expr) => {
                    Self::compare_arithmetic(expr, i, j, results, per_result_let)
                }
            };

            let is_similarity = matches!(
                &ob.expr,
                OrderByExpr::Similarity(_) | OrderByExpr::SimilarityBare
            );
            let directed_cmp =
                Self::apply_sort_direction(cmp, ob.descending, is_similarity, higher_is_better);
            if directed_cmp != Ordering::Equal {
                return directed_cmp;
            }
        }
        Ordering::Equal
    }

    /// Compares a payload field value between two results.
    fn compare_payload_field(
        field_name: &str,
        i: usize,
        j: usize,
        results: &[SearchResult],
    ) -> Ordering {
        let val_i = results[i]
            .point
            .payload
            .as_ref()
            .and_then(|p| p.get(field_name));
        let val_j = results[j]
            .point
            .payload
            .as_ref()
            .and_then(|p| p.get(field_name));
        compare_json_values(val_i, val_j)
    }

    /// Compares a field name, checking LET bindings first, then payload.
    fn compare_field_or_let(
        field_name: &str,
        i: usize,
        j: usize,
        results: &[SearchResult],
        per_result_let: &[Vec<(String, f32)>],
    ) -> Ordering {
        if let (Some(let_i), Some(let_j)) = (per_result_let.get(i), per_result_let.get(j)) {
            if let Some(vi) = let_i.iter().find(|(k, _)| k == field_name).map(|(_, v)| *v) {
                let vj = let_j
                    .iter()
                    .find(|(k, _)| k == field_name)
                    .map_or(0.0, |(_, v)| *v);
                return vi.total_cmp(&vj);
            }
        }
        Self::compare_payload_field(field_name, i, j, results)
    }

    /// Compares two results by an arithmetic expression with full context.
    fn compare_arithmetic(
        expr: &crate::velesql::ArithmeticExpr,
        i: usize,
        j: usize,
        results: &[SearchResult],
        per_result_let: &[Vec<(String, f32)>],
    ) -> Ordering {
        let ctx_i = ScoreContext::with_let_bindings(
            results[i].score,
            results[i].point.payload.as_ref(),
            results[i].component_scores.as_deref(),
            per_result_let.get(i).map(Vec::as_slice),
        );
        let ctx_j = ScoreContext::with_let_bindings(
            results[j].score,
            results[j].point.payload.as_ref(),
            results[j].component_scores.as_deref(),
            per_result_let.get(j).map(Vec::as_slice),
        );
        let val_i = evaluate_arithmetic(expr, &ctx_i);
        let val_j = evaluate_arithmetic(expr, &ctx_j);
        val_i.total_cmp(&val_j)
    }

    /// Applies ASC/DESC direction, accounting for distance metric inversion.
    fn apply_sort_direction(
        cmp: Ordering,
        descending: bool,
        is_similarity: bool,
        higher_is_better: bool,
    ) -> Ordering {
        if descending {
            if is_similarity && !higher_is_better {
                cmp
            } else {
                cmp.reverse()
            }
        } else if is_similarity && !higher_is_better {
            cmp.reverse()
        } else {
            cmp
        }
    }
}

/// Context for evaluating arithmetic ORDER BY expressions (EPIC-042).
///
/// Holds the pre-computed search score, optional per-component score breakdown,
/// LET bindings (v1.10), and optional payload for variable resolution.
pub(crate) struct ScoreContext<'a> {
    /// Pre-computed search score (vector similarity or fused score).
    search_score: f32,
    /// Payload fields for variable resolution.
    payload: Option<&'a serde_json::Value>,
    /// Optional per-component scores from hybrid search (v1.10+).
    ///
    /// When present, built-in score variables (`vector_score`, `bm25_score`, etc.)
    /// resolve to their individual component values instead of the fused score.
    component_scores: Option<&'a [(String, f32)]>,
    /// Pre-evaluated LET binding values (VelesQL v1.10 Phase 3).
    ///
    /// Resolution priority: LET bindings > component_scores > search_score > payload.
    let_bindings: Option<&'a [(String, f32)]>,
}

impl<'a> ScoreContext<'a> {
    /// Creates a new score context without component scores (backward compat).
    ///
    /// Used by tests and simple code paths where component scores are not available.
    #[allow(dead_code)] // Used by ordering_tests and component_scores_tests.
    pub(crate) fn new(search_score: f32, payload: Option<&'a serde_json::Value>) -> Self {
        Self {
            search_score,
            payload,
            component_scores: None,
            let_bindings: None,
        }
    }

    /// Creates a score context with per-component score breakdown.
    #[allow(dead_code)] // Used by component_scores_tests.
    pub(crate) fn with_components(
        search_score: f32,
        payload: Option<&'a serde_json::Value>,
        component_scores: Option<&'a [(String, f32)]>,
    ) -> Self {
        Self {
            search_score,
            payload,
            component_scores,
            let_bindings: None,
        }
    }

    /// Creates a score context with LET bindings and component scores.
    pub(crate) fn with_let_bindings(
        search_score: f32,
        payload: Option<&'a serde_json::Value>,
        component_scores: Option<&'a [(String, f32)]>,
        let_bindings: Option<&'a [(String, f32)]>,
    ) -> Self {
        Self {
            search_score,
            payload,
            component_scores,
            let_bindings,
        }
    }

    /// Resolves a variable name to a numeric value.
    ///
    /// Resolution priority:
    /// 1. LET bindings (highest — user-defined score aliases).
    /// 2. Built-in component scores (`vector_score`, `bm25_score`, etc.).
    /// 3. `search_score` (fused/primary score) for built-in names.
    /// 4. Payload fields for non-built-in names.
    ///
    /// `fused_score` and `similarity` always resolve to `search_score` (they
    /// represent the combined result, not an individual component).
    fn resolve_variable(&self, name: &str) -> f32 {
        // Priority 1: LET bindings override everything.
        if let Some(val) = self.lookup_let_binding(name) {
            return val;
        }
        match name {
            // fused_score and similarity always use the primary fused score.
            "fused_score" | "similarity" => self.search_score,
            // Component-aware built-ins: check component_scores first.
            "vector_score" | "graph_score" | "bm25_score" | "sparse_score" => {
                self.lookup_component(name).unwrap_or(self.search_score)
            }
            _ => self.resolve_payload_variable(name),
        }
    }

    /// Looks up a named LET binding, returning `None` if absent.
    fn lookup_let_binding(&self, name: &str) -> Option<f32> {
        self.let_bindings?
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| *v)
    }

    /// Looks up a named component score, returning `None` if absent.
    fn lookup_component(&self, name: &str) -> Option<f32> {
        self.component_scores?
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| *v)
    }

    /// Resolves a variable name from the payload.
    fn resolve_payload_variable(&self, name: &str) -> f32 {
        self.payload
            .and_then(|p| p.get(name))
            .and_then(serde_json::Value::as_f64)
            .map_or(0.0, |v| {
                #[allow(clippy::cast_possible_truncation)]
                // Reason: payload values are user-defined scores; f64→f32 precision loss is acceptable.
                {
                    v as f32
                }
            })
    }
}

/// Evaluates all LET bindings in declaration order for a single result.
///
/// Each binding can reference earlier bindings, component scores, or the
/// search score. The returned vec contains `(name, value)` pairs in order.
pub(crate) fn evaluate_let_bindings(
    bindings: &[crate::velesql::LetBinding],
    search_score: f32,
    payload: Option<&serde_json::Value>,
    component_scores: Option<&[(String, f32)]>,
) -> Vec<(String, f32)> {
    let mut evaluated: Vec<(String, f32)> = Vec::with_capacity(bindings.len());
    for binding in bindings {
        let ctx = ScoreContext::with_let_bindings(
            search_score,
            payload,
            component_scores,
            Some(&evaluated),
        );
        let value = evaluate_arithmetic(&binding.expr, &ctx);
        evaluated.push((binding.name.clone(), value));
    }
    evaluated
}

/// Maximum recursion depth for arithmetic expression evaluation.
/// Matches `DEFAULT_MAX_AST_DEPTH` (64) from validation.
const MAX_ARITHMETIC_DEPTH: u8 = 64;

/// Evaluates an arithmetic expression against a score context (EPIC-042).
///
/// Division by zero returns `0.0` (safe default for sorting).
/// Recursion depth is capped at [`MAX_ARITHMETIC_DEPTH`] to prevent stack overflow.
pub(crate) fn evaluate_arithmetic(expr: &ArithmeticExpr, ctx: &ScoreContext<'_>) -> f32 {
    evaluate_arithmetic_inner(expr, ctx, 0)
}

/// Inner recursive evaluator with depth tracking.
fn evaluate_arithmetic_inner(expr: &ArithmeticExpr, ctx: &ScoreContext<'_>, depth: u8) -> f32 {
    if depth >= MAX_ARITHMETIC_DEPTH {
        return 0.0;
    }
    match expr {
        ArithmeticExpr::Literal(v) => {
            #[allow(clippy::cast_possible_truncation)]
            // Reason: arithmetic literals are user-defined weights; f64→f32 precision loss is acceptable.
            {
                *v as f32
            }
        }
        ArithmeticExpr::Variable(name) => ctx.resolve_variable(name),
        // Only bare similarity() passes validation inside arithmetic expressions.
        // Parameterized similarity(field, $vec) is rejected at validation time (V008).
        ArithmeticExpr::Similarity(_) => ctx.search_score,
        ArithmeticExpr::BinaryOp { left, op, right } => {
            let l = evaluate_arithmetic_inner(left, ctx, depth + 1);
            let r = evaluate_arithmetic_inner(right, ctx, depth + 1);
            match op {
                ArithmeticOp::Add => l + r,
                ArithmeticOp::Sub => l - r,
                ArithmeticOp::Mul => l * r,
                ArithmeticOp::Div => {
                    if r.abs() > f32::EPSILON {
                        l / r
                    } else {
                        0.0
                    }
                }
            }
        }
    }
}
