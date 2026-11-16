//! ORDER BY execution for the WASM VelesQL executor (S4-13, #8).
//!
//! Supports ordering on any payload column (not just `id` / `score`),
//! multi-key sort, ASC/DESC per key, core's value order
//! (`velesql::compare_json_values`: nulls last in ASC, first in DESC), and arithmetic
//! expressions over the per-row score + payload.
//!
//! Sorting operates on core [`SearchResult`] values so the same rows can be
//! projected through velesdb-core's projection engine afterwards (#3b). A
//! score variable (`score`, `vector_score`, ...) and an arithmetic key resolve
//! through core's own [`velesdb_core::velesql::ScoreContext`] — the same
//! resolution order (LET, search score, component scores, `id`, payload) as
//! core's ORDER BY, so no name reads differently in the browser.
//!
//! Genuinely unevaluable forms — `similarity(field, $v)` against a named
//! vector, which WASM does not store, and aggregate ORDER BY outside an
//! aggregation pipeline — are **rejected loudly** rather than silently
//! degrading to scan order, matching the MATCH path (#8a).

use std::cmp::Ordering;

use velesdb_core::point::SearchResult;
use velesdb_core::velesql::{
    evaluate_arithmetic, is_builtin_score_variable, ArithmeticExpr, OrderByExpr, ScoreContext,
    SelectOrderBy, SelectStatement,
};

/// Sorts a result set in place according to the SELECT's ORDER BY clause.
///
/// Returns `Err` for ORDER BY shapes WASM cannot evaluate (named-vector
/// `similarity(field, $v)` or aggregate expressions outside aggregation), so
/// the executor fails loud instead of returning scan order (#8a).
/// `higher_is_better` is the collection metric's direction: `similarity()`
/// sorts most similar first under `DESC` whatever the metric, as in core
/// (`SelectOrderBy::directed`).
pub(crate) fn sort_rows(
    stmt: &SelectStatement,
    rows: &mut [SearchResult],
    higher_is_better: bool,
) -> Result<(), String> {
    let Some(specs) = stmt.order_by.as_ref() else {
        return Ok(());
    };
    if specs.is_empty() {
        return Ok(());
    }
    validate_order_by(specs)?;
    rows.sort_by(|a, b| compare_rows(a, b, specs, higher_is_better));
    Ok(())
}

/// Rejects ORDER BY expressions WASM cannot evaluate before sorting begins.
fn validate_order_by(specs: &[SelectOrderBy]) -> Result<(), String> {
    for spec in specs {
        match &spec.expr {
            OrderByExpr::Similarity(_) => {
                return Err(
                    "ORDER BY similarity(field, $vec) is not supported in WASM: named/secondary \
                     vectors are not stored. Use ORDER BY similarity() (the search score) or a \
                     payload column."
                        .to_string(),
                );
            }
            OrderByExpr::Aggregate(_) => {
                return Err(
                    "ORDER BY aggregate is only valid in an aggregation (GROUP BY) query"
                        .to_string(),
                );
            }
            OrderByExpr::Field(_) | OrderByExpr::SimilarityBare | OrderByExpr::Arithmetic(_) => {}
            // `OrderByExpr` is non_exhaustive: any future variant is unproven
            // on the WASM surface, so reject rather than silently no-op.
            _ => return Err("ORDER BY expression is not supported in WASM".to_string()),
        }
    }
    Ok(())
}

/// Strict total order used by `sort_by` over a multi-key ORDER BY.
fn compare_rows(
    a: &SearchResult,
    b: &SearchResult,
    specs: &[SelectOrderBy],
    higher_is_better: bool,
) -> Ordering {
    for spec in specs {
        let ord = spec.directed(compare_with_spec(a, b, spec), higher_is_better);
        if ord != Ordering::Equal {
            return ord;
        }
    }
    // Deterministic tie-break by ascending id, matching core's ORDER BY.
    a.point.id.cmp(&b.point.id)
}

/// Compares two rows on one ORDER BY item, ascending.
fn compare_with_spec(a: &SearchResult, b: &SearchResult, spec: &SelectOrderBy) -> Ordering {
    match &spec.expr {
        OrderByExpr::Field(name) => compare_field(a, b, name),
        OrderByExpr::SimilarityBare => compare_scores(a, b),
        OrderByExpr::Arithmetic(expr) => {
            let va = eval_arithmetic(expr, a);
            let vb = eval_arithmetic(expr, b);
            compare_f32(va, vb)
        }
        // Similarity / Aggregate were rejected by `validate_order_by`; treat
        // as equal defensively (the non_exhaustive enum may add variants).
        _ => Ordering::Equal,
    }
}

/// Compares a pair of rows by the given column: `id` is the point id, a
/// score variable resolves as core resolves it, any other name is a payload
/// field.
fn compare_field(a: &SearchResult, b: &SearchResult, name: &str) -> Ordering {
    if name == "id" {
        return a.point.id.cmp(&b.point.id);
    }
    if is_builtin_score_variable(name) {
        return compare_f32(
            score_context(a).resolve_variable(name),
            score_context(b).resolve_variable(name),
        );
    }
    let va = extract_payload_field(a, name);
    let vb = extract_payload_field(b, name);
    velesdb_core::velesql::compare_json_values(va.as_ref(), vb.as_ref())
}

/// Compares two rows by search score (NaN-safe, NaN sorts last in ASC).
fn compare_scores(a: &SearchResult, b: &SearchResult) -> Ordering {
    compare_f32(a.score, b.score)
}

/// NaN-safe f32 comparison (NaN sorts last in ASC).
fn compare_f32(a: f32, b: f32) -> Ordering {
    match (a.is_nan(), b.is_nan()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        (false, false) => a.partial_cmp(&b).unwrap_or(Ordering::Equal),
    }
}

/// Pulls a column from the payload: the key itself first (a group row stores
/// `first_meta.title` flat), then the dot-nested path — core's `row_field`.
fn extract_payload_field(result: &SearchResult, column: &str) -> Option<serde_json::Value> {
    result
        .point
        .payload
        .as_ref()
        .and_then(|p| velesdb_core::velesql::row_field(p, column).cloned())
}

/// Evaluates an arithmetic ORDER BY expression for a single row through
/// core's evaluator.
fn eval_arithmetic(expr: &ArithmeticExpr, result: &SearchResult) -> f32 {
    evaluate_arithmetic(expr, &score_context(result))
}

/// What a row's score variables read: its search score, its component
/// breakdown (a NEAR row carries `vector_score`, as in core) and its id.
fn score_context(result: &SearchResult) -> ScoreContext<'_> {
    ScoreContext::with_let_bindings(
        result.score,
        result.point.payload.as_ref(),
        result.component_scores.as_deref(),
        None,
    )
    .for_point(result.point.id)
}

#[cfg(test)]
#[path = "velesql_orderby_tests.rs"]
mod tests;
