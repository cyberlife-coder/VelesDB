//! ORDER BY execution for the WASM VelesQL executor (S4-13, #8).
//!
//! Supports ordering on any payload column (not just `id` / `score`),
//! multi-key sort, ASC/DESC per key, explicit null handling (nulls sort last
//! in ASC and first in DESC, matching Postgres default), and arithmetic
//! expressions over the per-row score + payload.
//!
//! Sorting operates on core [`SearchResult`] values so the same rows can be
//! projected through velesdb-core's projection engine afterwards (#3b). The
//! arithmetic evaluator mirrors core's `ordering::evaluate_arithmetic`: score
//! variables resolve to the row's search score, payload variables to their
//! numeric value (0.0 when missing/non-numeric), and division by zero yields
//! 0.0.
//!
//! Genuinely unevaluable forms — `similarity(field, $v)` against a named
//! vector, which WASM does not store, and aggregate ORDER BY outside an
//! aggregation pipeline — are **rejected loudly** rather than silently
//! degrading to scan order, matching the MATCH path (#8a).

use std::cmp::Ordering;

use velesdb_core::point::SearchResult;
use velesdb_core::velesql::{
    ArithmeticExpr, ArithmeticOp, OrderByExpr, SelectOrderBy, SelectStatement,
};

use crate::velesql_value::json_values_cmp;

/// Maximum recursion depth for arithmetic evaluation (mirrors core's
/// `MAX_ARITHMETIC_DEPTH`).
const MAX_ARITHMETIC_DEPTH: u8 = 64;

/// Sorts a result set in place according to the SELECT's ORDER BY clause.
///
/// Returns `Err` for ORDER BY shapes WASM cannot evaluate (named-vector
/// `similarity(field, $v)` or aggregate expressions outside aggregation), so
/// the executor fails loud instead of returning scan order (#8a).
pub(crate) fn sort_rows(stmt: &SelectStatement, rows: &mut [SearchResult]) -> Result<(), String> {
    let Some(specs) = stmt.order_by.as_ref() else {
        return Ok(());
    };
    if specs.is_empty() {
        return Ok(());
    }
    validate_order_by(specs)?;
    rows.sort_by(|a, b| compare_rows(a, b, specs));
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
fn compare_rows(a: &SearchResult, b: &SearchResult, specs: &[SelectOrderBy]) -> Ordering {
    for spec in specs {
        let ord = compare_with_spec(a, b, spec);
        if ord != Ordering::Equal {
            return ord;
        }
    }
    // Deterministic tie-break by ascending id, matching core's ORDER BY.
    a.point.id.cmp(&b.point.id)
}

fn compare_with_spec(a: &SearchResult, b: &SearchResult, spec: &SelectOrderBy) -> Ordering {
    let ord = match &spec.expr {
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
    };
    if spec.descending {
        ord.reverse()
    } else {
        ord
    }
}

/// Compares a pair of rows by the given column. `id` and `score` are
/// resolved from their dedicated fields; everything else from the payload.
fn compare_field(a: &SearchResult, b: &SearchResult, name: &str) -> Ordering {
    if name == "id" {
        return a.point.id.cmp(&b.point.id);
    }
    if name == "score" {
        return compare_scores(a, b);
    }
    let va = extract_payload_field(a, name);
    let vb = extract_payload_field(b, name);
    compare_json_with_nulls(va.as_ref(), vb.as_ref())
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

/// Pulls a column from the payload (supports dot-nested paths).
fn extract_payload_field(result: &SearchResult, column: &str) -> Option<serde_json::Value> {
    result
        .point
        .payload
        .as_ref()
        .and_then(|p| crate::filter::get_nested_field(p, column).cloned())
}

/// Evaluates an arithmetic ORDER BY expression for a single row, mirroring
/// core's `ordering::evaluate_arithmetic` semantics.
fn eval_arithmetic(expr: &ArithmeticExpr, result: &SearchResult) -> f32 {
    eval_arithmetic_inner(expr, result, 0)
}

fn eval_arithmetic_inner(expr: &ArithmeticExpr, result: &SearchResult, depth: u8) -> f32 {
    if depth >= MAX_ARITHMETIC_DEPTH {
        return 0.0;
    }
    match expr {
        #[allow(clippy::cast_possible_truncation)]
        ArithmeticExpr::Literal(v) => *v as f32,
        ArithmeticExpr::Variable(name) => resolve_variable(name, result),
        // Only bare similarity() reaches arithmetic (validation rejects the
        // named form); it resolves to the row search score.
        ArithmeticExpr::Similarity(_) => result.score,
        ArithmeticExpr::BinaryOp { left, op, right } => {
            let l = eval_arithmetic_inner(left, result, depth + 1);
            let r = eval_arithmetic_inner(right, result, depth + 1);
            apply_op(*op, l, r)
        }
        // `ArithmeticExpr` is non_exhaustive; an unknown future variant has no
        // defined WASM semantics, so contribute a neutral 0.0 to the sort key.
        _ => 0.0,
    }
}

/// Applies a binary arithmetic operator; division by zero yields 0.0.
fn apply_op(op: ArithmeticOp, l: f32, r: f32) -> f32 {
    match op {
        ArithmeticOp::Add => l + r,
        ArithmeticOp::Sub => l - r,
        ArithmeticOp::Mul => l * r,
        ArithmeticOp::Div => {
            if r == 0.0 {
                0.0
            } else {
                l / r
            }
        }
        // `ArithmeticOp` is non_exhaustive; a future operator has no defined
        // WASM semantics, so fall back to the left operand rather than guess.
        _ => l,
    }
}

/// Resolves a variable name to a numeric value for arithmetic ORDER BY.
///
/// Score built-ins (`score`, `similarity`, `vector_score`, …) resolve to the
/// row search score — WASM scores only the primary vector, so absent
/// component scores fall back to the search score (core's legacy/untagged
/// path). Any other name is a payload field (0.0 when missing/non-numeric).
fn resolve_variable(name: &str, result: &SearchResult) -> f32 {
    match name {
        "score" | "similarity" | "fused_score" | "vector_score" | "graph_score" | "bm25_score"
        | "sparse_score" => result.score,
        _ => extract_payload_field(result, name)
            .as_ref()
            .and_then(serde_json::Value::as_f64)
            .map_or(0.0, |v| {
                #[allow(clippy::cast_possible_truncation)]
                {
                    v as f32
                }
            }),
    }
}

/// Postgres-style NULL ordering: null sorts AFTER non-null in ASC. Since we
/// always compute ASC first and reverse for DESC via `compare_with_spec`,
/// producing "null > non-null" here gives "NULLS LAST in ASC" / "NULLS FIRST
/// in DESC".
///
/// Shared with the MATCH `ORDER BY` path (`velesql_match_orderby`) so node and
/// row ordering apply identical NULL semantics.
pub(crate) fn compare_json_with_nulls(
    left: Option<&serde_json::Value>,
    right: Option<&serde_json::Value>,
) -> Ordering {
    let left_null = left.is_none_or(serde_json::Value::is_null);
    let right_null = right.is_none_or(serde_json::Value::is_null);
    match (left_null, right_null) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        (false, false) => match (left, right) {
            (Some(l), Some(r)) => json_values_cmp(l, r).unwrap_or(Ordering::Equal),
            _ => Ordering::Equal,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use velesdb_core::point::Point;
    use velesdb_core::velesql::{ArithmeticExpr, ArithmeticOp, OrderByExpr, SelectOrderBy};

    fn mk(id: u64, score: f32, payload: serde_json::Value) -> SearchResult {
        SearchResult::new(Point::new(id, Vec::new(), Some(payload)), score)
    }

    fn by_field(name: &str, desc: bool) -> Vec<SelectOrderBy> {
        vec![SelectOrderBy {
            expr: OrderByExpr::Field(name.to_string()),
            descending: desc,
        }]
    }

    fn sort(stmt_order: Vec<SelectOrderBy>, rows: &mut [SearchResult]) {
        let mut stmt = SelectStatement::empty();
        stmt.order_by = Some(stmt_order);
        sort_rows(&stmt, rows).expect("test: sort");
    }

    #[test]
    fn test_sort_by_id_asc() {
        let mut rows = vec![
            mk(3, 0.0, serde_json::json!({})),
            mk(1, 0.0, serde_json::json!({})),
            mk(2, 0.0, serde_json::json!({})),
        ];
        sort(by_field("id", false), &mut rows);
        assert_eq!(rows[0].point.id, 1);
        assert_eq!(rows[2].point.id, 3);
    }

    #[test]
    fn test_sort_by_payload_column_desc() {
        let mut rows = vec![
            mk(1, 0.0, serde_json::json!({"price": 20})),
            mk(2, 0.0, serde_json::json!({"price": 10})),
            mk(3, 0.0, serde_json::json!({"price": 30})),
        ];
        sort(by_field("price", true), &mut rows);
        assert_eq!(rows[0].point.id, 3);
        assert_eq!(rows[2].point.id, 2);
    }

    #[test]
    fn test_sort_nulls_last_asc() {
        let mut rows = vec![
            mk(1, 0.0, serde_json::json!({"x": 5})),
            mk(2, 0.0, serde_json::json!({})),
            mk(3, 0.0, serde_json::json!({"x": 1})),
        ];
        sort(by_field("x", false), &mut rows);
        assert_eq!(rows[0].point.id, 3);
        assert_eq!(rows[2].point.id, 2);
    }

    #[test]
    fn test_sort_by_similarity_bare_desc() {
        let mut rows = vec![
            mk(1, 0.1, serde_json::json!({})),
            mk(2, 0.9, serde_json::json!({})),
            mk(3, 0.5, serde_json::json!({})),
        ];
        sort(
            vec![SelectOrderBy {
                expr: OrderByExpr::SimilarityBare,
                descending: true,
            }],
            &mut rows,
        );
        assert_eq!(rows[0].point.id, 2);
        assert_eq!(rows[2].point.id, 1);
    }

    #[test]
    fn test_sort_by_arithmetic_formula() {
        // ORDER BY (price - 2*score) ASC.
        let expr = ArithmeticExpr::BinaryOp {
            left: Box::new(ArithmeticExpr::Variable("price".to_string())),
            op: ArithmeticOp::Sub,
            right: Box::new(ArithmeticExpr::BinaryOp {
                left: Box::new(ArithmeticExpr::Literal(2.0)),
                op: ArithmeticOp::Mul,
                right: Box::new(ArithmeticExpr::Variable("score".to_string())),
            }),
        };
        let mut rows = vec![
            mk(1, 1.0, serde_json::json!({"price": 10})), // 10 - 2 = 8
            mk(2, 0.0, serde_json::json!({"price": 1})),  // 1
            mk(3, 0.0, serde_json::json!({"price": 30})), // 30
        ];
        sort(
            vec![SelectOrderBy {
                expr: OrderByExpr::Arithmetic(expr),
                descending: false,
            }],
            &mut rows,
        );
        assert_eq!(rows[0].point.id, 2); // 1
        assert_eq!(rows[1].point.id, 1); // 8
        assert_eq!(rows[2].point.id, 3); // 30
    }

    #[test]
    fn test_sort_named_similarity_is_rejected() {
        use velesdb_core::velesql::{SimilarityOrderBy, VectorExpr};
        let mut rows = vec![mk(1, 0.5, serde_json::json!({}))];
        let mut stmt = SelectStatement::empty();
        stmt.order_by = Some(vec![SelectOrderBy {
            expr: OrderByExpr::Similarity(SimilarityOrderBy {
                field: "image_vec".to_string(),
                vector: VectorExpr::Parameter("q".to_string()),
            }),
            descending: true,
        }]);
        let err = sort_rows(&stmt, &mut rows);
        assert!(err.is_err());
    }

    #[test]
    fn test_division_by_zero_yields_zero() {
        assert_eq!(apply_op(ArithmeticOp::Div, 5.0, 0.0), 0.0);
    }
}
