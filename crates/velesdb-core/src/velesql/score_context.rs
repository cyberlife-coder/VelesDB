//! The scores an ORDER BY or a LET reads, as every engine resolves them —
//! core's SELECT and MATCH ordering and the WASM executor alike.
//!
//! A variable resolves, in order, to a LET binding of that name; `score`,
//! `fused_score` or `similarity` (the row's search score); a component score
//! (`vector_score`, `bm25_score`, ...: the result's breakdown, `0.0` for a
//! component a tagged result lacks, the search score on an untagged one);
//! `id` (the point id); else a numeric payload field (`0.0` when absent).

use crate::velesql::{ArithmeticExpr, ArithmeticOp};

/// Whether `name` is a built-in score variable resolved from the component-score
/// breakdown (rather than a payload field), matching the names
/// `ScoreContext::resolve_variable` recognizes. Bare such names in `ORDER BY`
/// route through score resolution so e.g. `ORDER BY sparse_score DESC` ranks.
/// `score` is the row's search score: a reserved word, not a payload field
/// (VELESQL_SPEC, reserved keywords).
#[must_use]
pub fn is_builtin_score_variable(name: &str) -> bool {
    matches!(
        name,
        "score" | "vector_score" | "graph_score" | "bm25_score" | "sparse_score" | "fused_score"
    )
}

/// Context for evaluating arithmetic ORDER BY expressions (EPIC-042).
///
/// Holds the pre-computed search score, optional per-component score breakdown,
/// LET bindings (v1.10), and optional payload for variable resolution.
pub struct ScoreContext<'a> {
    /// Pre-computed search score (vector similarity or fused score).
    search_score: f32,
    /// Payload fields for variable resolution.
    payload: Option<&'a serde_json::Value>,
    /// Optional per-component scores from hybrid search (v1.10+).
    ///
    /// When present, built-in score variables (`vector_score`, `bm25_score`, etc.)
    /// resolve to their individual component values instead of the fused score.
    component_scores: Option<&'a [(&'static str, f32)]>,
    /// Pre-evaluated LET binding values (VelesQL v1.10 Phase 3).
    ///
    /// Resolution priority: LET bindings > component_scores > search_score > payload.
    let_bindings: Option<&'a [(String, f32)]>,
    /// The point id, which `id` reads, as in every clause; `None` where the
    /// caller has no point (tests), and `id` then reads the payload.
    point_id: Option<u64>,
}

impl<'a> ScoreContext<'a> {
    /// Creates a new score context without component scores (backward compat).
    ///
    /// Used by tests and simple code paths where component scores are not available.
    #[must_use]
    pub fn new(search_score: f32, payload: Option<&'a serde_json::Value>) -> Self {
        Self {
            search_score,
            payload,
            component_scores: None,
            let_bindings: None,
            point_id: None,
        }
    }

    /// Creates a score context with per-component score breakdown.
    #[cfg(test)]
    #[must_use]
    pub fn with_components(
        search_score: f32,
        payload: Option<&'a serde_json::Value>,
        component_scores: Option<&'a [(&'static str, f32)]>,
    ) -> Self {
        Self {
            search_score,
            payload,
            component_scores,
            let_bindings: None,
            point_id: None,
        }
    }

    /// Creates a score context with LET bindings and component scores.
    #[must_use]
    pub fn with_let_bindings(
        search_score: f32,
        payload: Option<&'a serde_json::Value>,
        component_scores: Option<&'a [(&'static str, f32)]>,
        let_bindings: Option<&'a [(String, f32)]>,
    ) -> Self {
        Self {
            search_score,
            payload,
            component_scores,
            let_bindings,
            point_id: None,
        }
    }

    /// This context for the point `id`, which the variable `id` reads.
    #[must_use]
    pub const fn for_point(mut self, id: u64) -> Self {
        self.point_id = Some(id);
        self
    }

    /// Resolves a variable name to a numeric value.
    ///
    /// Resolution priority:
    /// 1. LET bindings (highest — user-defined score aliases).
    /// 2. Built-in component scores (`vector_score`, `bm25_score`, etc.).
    /// 3. For a built-in absent from a *tagged* result (component_scores `Some`),
    ///    the component defaults to `0.0` (per VELESQL_SPEC); on an *untagged*
    ///    result (component_scores `None`) it falls back to `search_score`.
    /// 4. Payload fields for non-built-in names.
    ///
    /// `fused_score` and `similarity` always resolve to `search_score` (they
    /// represent the combined result, not an individual component).
    ///
    /// `graph_score` is reserved: it is never populated as a component, so it
    /// resolves to `0.0` on tagged results (and to `search_score` only on the
    /// legacy untagged path).
    #[must_use]
    pub fn resolve_variable(&self, name: &str) -> f32 {
        // Priority 1: LET bindings override everything.
        if let Some(val) = self.lookup_let_binding(name) {
            return val;
        }
        match name {
            // score, fused_score and similarity always use the primary score.
            "score" | "fused_score" | "similarity" => self.search_score,
            // Component-aware built-ins: a tagged result (component_scores Some)
            // defaults an absent component to 0.0; only the untagged legacy path
            // falls back to the primary search_score.
            "vector_score" | "graph_score" | "bm25_score" | "sparse_score" => self
                .lookup_component(name)
                .unwrap_or_else(|| self.absent_component_default()),
            _ => self.resolve_payload_variable(name),
        }
    }

    /// Default for a built-in component absent from this result: `0.0` when the
    /// result carries a component breakdown (`component_scores` is `Some`),
    /// otherwise the primary `search_score` (legacy untagged compatibility).
    fn absent_component_default(&self) -> f32 {
        if self.component_scores.is_some() {
            0.0
        } else {
            self.search_score
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
            .find(|(k, _)| *k == name)
            .map(|(_, v)| *v)
    }

    /// Resolves a variable name from the payload; `id` is the point id.
    fn resolve_payload_variable(&self, name: &str) -> f32 {
        if let (Some(id), "id") = (self.point_id, name) {
            // Reason: an arithmetic score is an f32, ids included.
            #[allow(clippy::cast_precision_loss)]
            return id as f32;
        }
        self.payload
            .and_then(|p| crate::velesql::row_field(p, name))
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
#[must_use]
pub fn evaluate_let_bindings(
    bindings: &[crate::velesql::LetBinding],
    search_score: f32,
    payload: Option<&serde_json::Value>,
    component_scores: Option<&[(&'static str, f32)]>,
    point_id: Option<u64>,
) -> Vec<(String, f32)> {
    let mut evaluated: Vec<(String, f32)> = Vec::with_capacity(bindings.len());
    for binding in bindings {
        let mut ctx = ScoreContext::with_let_bindings(
            search_score,
            payload,
            component_scores,
            Some(&evaluated),
        );
        if let Some(id) = point_id {
            ctx = ctx.for_point(id);
        }
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
#[must_use]
pub fn evaluate_arithmetic(expr: &ArithmeticExpr, ctx: &ScoreContext<'_>) -> f32 {
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
