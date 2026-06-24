//! MATCH `RETURN ... ORDER BY` sorting for the WASM executor.
//!
//! Mirrors core's `order_match_results` semantics
//! (`match_exec/order_by.rs` + `similarity.rs`): a deterministic baseline keyed
//! by the row's bound node ids, then each ORDER BY key applied as a STABLE sort
//! least-significant-first (`.rev()`), so multi-key ordering is correct and ties
//! resolve to a total, deterministic order. Sorting happens BEFORE the LIMIT
//! truncation (the caller collects the full candidate set first).
//!
//! Supported keys (the forms the WASM in-memory graph path can evaluate
//! exactly): `depth` (constant within a single pattern, so a defined no-op) and
//! an `alias.property` path. `similarity()` / `similarity(field, $v)`,
//! arithmetic, and aggregate expressions need vector / score-context evaluation
//! the WASM MATCH path does not materialize (it performs no vector scoring), so
//! they are rejected with a clear error rather than silently mis-ordered.

use std::cmp::Ordering;

use velesdb_core::velesql::{OrderByExpr, OrderByItem};

use crate::velesql_result::QueryResultRow;

/// A MATCH result row paired with the data needed to order it.
pub(crate) struct MatchCandidate {
    /// The row's bound node ids in pattern order (`[a]`, `[a, b]`, `[a, b, c]`).
    /// Each matched row has a distinct tuple, so this is a per-row-unique
    /// deterministic tie-break baseline (a TOTAL order) — mirroring core keying
    /// its baseline on the full match identity rather than the anchor alone
    /// (anchor `a` repeats across the many `b`/`c` of a star pattern).
    pub baseline: Vec<u64>,
    /// Alias-keyed row JSON (e.g. `{"a": {...}, "b": {...}}`), used to resolve
    /// `alias.property` ORDER BY keys via `get_nested_field`.
    pub value: serde_json::Value,
    /// The serialized row returned to the caller after sorting.
    pub row: QueryResultRow,
}

/// Sorts MATCH candidates in place per the RETURN `ORDER BY`, then truncates to
/// `limit`. With no ORDER BY, only the limit is applied (traversal order
/// preserved) — matching the prior behavior for that case.
///
/// # Errors
///
/// Returns an error for ORDER BY forms the WASM MATCH path cannot evaluate
/// (`similarity()`, `similarity(field, $v)`, arithmetic, aggregate).
pub(crate) fn order_and_limit(
    candidates: &mut Vec<MatchCandidate>,
    order_by: Option<&[OrderByItem]>,
    limit: Option<u64>,
) -> Result<(), String> {
    if let Some(items) = order_by {
        // Deterministic baseline: the bound-node-id tuple is unique per row, so
        // it gives the stable per-key sorts below a total tie-break order.
        candidates.sort_by(|a, b| a.baseline.cmp(&b.baseline));
        // Apply least-significant key first so the most-significant wins.
        for item in items.iter().rev() {
            sort_by_key(candidates, &item.expr, item.descending)?;
        }
    }
    if let Some(limit) = limit {
        let limit = usize::try_from(limit).unwrap_or(usize::MAX);
        candidates.truncate(limit);
    }
    Ok(())
}

/// Applies one STABLE ORDER BY key.
fn sort_by_key(
    candidates: &mut [MatchCandidate],
    expr: &OrderByExpr,
    descending: bool,
) -> Result<(), String> {
    match expr {
        OrderByExpr::Field(f) if f == "depth" => {
            // WASM patterns have a fixed depth per query, so this is a defined
            // no-op within a single pattern — kept for parity, not silently
            // dropped.
            Ok(())
        }
        OrderByExpr::Field(f) => sort_by_property(candidates, f, descending),
        OrderByExpr::SimilarityBare => {
            reject_unsupported("similarity() (the browser MATCH path performs no vector scoring)")
        }
        OrderByExpr::Similarity(_) => reject_unsupported("similarity(field, $v)"),
        OrderByExpr::Arithmetic(_) => reject_unsupported("arithmetic expressions"),
        _ => reject_unsupported("aggregate expressions"),
    }
}

/// Uniform rejection for ORDER BY forms the WASM MATCH path cannot evaluate
/// (it materializes no vector scores; only `depth` and `alias.property` work).
fn reject_unsupported(form: &str) -> Result<(), String> {
    Err(format!(
        "MATCH ORDER BY {form} is not supported in WASM (use depth or alias.property)"
    ))
}

/// Sorts by an `alias.property` JSON path (dot-nested), nulls last in ASC.
fn sort_by_property(
    candidates: &mut [MatchCandidate],
    path: &str,
    descending: bool,
) -> Result<(), String> {
    if !path.contains('.') {
        return reject_unsupported(&format!("expression '{path}'"));
    }
    sort_stable(candidates, descending, |a, b| {
        let va = crate::filter::get_nested_field(&a.value, path);
        let vb = crate::filter::get_nested_field(&b.value, path);
        crate::velesql_orderby::compare_json_with_nulls(va, vb)
    });
    Ok(())
}

/// STABLE sort applying the comparison and the ASC/DESC direction.
fn sort_stable<F>(candidates: &mut [MatchCandidate], descending: bool, cmp: F)
where
    F: Fn(&MatchCandidate, &MatchCandidate) -> Ordering,
{
    candidates.sort_by(|a, b| {
        let ord = cmp(a, b);
        if descending {
            ord.reverse()
        } else {
            ord
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(node_id: u64, value: serde_json::Value) -> MatchCandidate {
        MatchCandidate {
            baseline: vec![node_id],
            row: QueryResultRow::synthetic(value.clone()).expect("test: row"),
            value,
        }
    }

    fn ids(c: &[MatchCandidate]) -> Vec<u64> {
        c.iter().map(|x| x.baseline[0]).collect()
    }

    #[test]
    fn test_order_by_property_asc_then_limit() {
        let mut c = vec![
            cand(1, serde_json::json!({"a": {"age": 30}})),
            cand(2, serde_json::json!({"a": {"age": 10}})),
            cand(3, serde_json::json!({"a": {"age": 20}})),
        ];
        let ob = vec![OrderByItem {
            expr: OrderByExpr::Field("a.age".to_string()),
            descending: false,
        }];
        order_and_limit(&mut c, Some(&ob), Some(2)).expect("test: order");
        // Sorted by age asc (10,20,30) THEN limited to 2 -> ids 2,3.
        assert_eq!(ids(&c), vec![2, 3]);
    }

    #[test]
    fn test_limit_before_sort_bug_regression() {
        // The bug: limit applied during collection would truncate to the
        // FIRST-seen rows (1,2) before sorting. Correct: keep smallest-age.
        let mut c = vec![
            cand(1, serde_json::json!({"a": {"age": 99}})),
            cand(2, serde_json::json!({"a": {"age": 98}})),
            cand(3, serde_json::json!({"a": {"age": 1}})),
        ];
        let ob = vec![OrderByItem {
            expr: OrderByExpr::Field("a.age".to_string()),
            descending: false,
        }];
        order_and_limit(&mut c, Some(&ob), Some(1)).expect("test: order");
        assert_eq!(ids(&c), vec![3], "smallest age survives, not first-seen");
    }

    #[test]
    fn test_order_by_property_desc() {
        let mut c = vec![
            cand(1, serde_json::json!({"a": {"age": 30}})),
            cand(2, serde_json::json!({"a": {"age": 10}})),
        ];
        let ob = vec![OrderByItem {
            expr: OrderByExpr::Field("a.age".to_string()),
            descending: true,
        }];
        order_and_limit(&mut c, Some(&ob), None).expect("test: order");
        assert_eq!(ids(&c), vec![1, 2]);
    }

    #[test]
    fn test_no_order_by_only_limits() {
        let mut c = vec![
            cand(5, serde_json::json!({"a": {}})),
            cand(3, serde_json::json!({"a": {}})),
            cand(9, serde_json::json!({"a": {}})),
        ];
        order_and_limit(&mut c, None, Some(2)).expect("test: order");
        // Traversal order preserved (no sort), just truncated.
        assert_eq!(ids(&c), vec![5, 3]);
    }

    #[test]
    fn test_tie_break_by_baseline() {
        let mut c = vec![
            cand(3, serde_json::json!({"a": {"k": 1}})),
            cand(1, serde_json::json!({"a": {"k": 1}})),
            cand(2, serde_json::json!({"a": {"k": 1}})),
        ];
        let ob = vec![OrderByItem {
            expr: OrderByExpr::Field("a.k".to_string()),
            descending: false,
        }];
        order_and_limit(&mut c, Some(&ob), None).expect("test: order");
        assert_eq!(
            ids(&c),
            vec![1, 2, 3],
            "all-equal keys tie-break by node-id baseline"
        );
    }

    #[test]
    fn test_multi_node_baseline_is_total_order() {
        // Two rows share anchor a=1 but differ in b; tied on the ORDER BY key
        // they must order deterministically by the full (a, b) tuple, not
        // collapse to anchor-only order (the parity defect this fixes).
        let row = |a: u64, b: u64| MatchCandidate {
            baseline: vec![a, b],
            row: QueryResultRow::synthetic(serde_json::json!({"a": {"k": 1}, "b": {}}))
                .expect("test: row"),
            value: serde_json::json!({"a": {"k": 1}, "b": {}}),
        };
        let mut c = vec![row(1, 9), row(1, 4)];
        let ob = vec![OrderByItem {
            expr: OrderByExpr::Field("a.k".to_string()),
            descending: false,
        }];
        order_and_limit(&mut c, Some(&ob), None).expect("test: order");
        assert_eq!(
            c.iter().map(|x| x.baseline.clone()).collect::<Vec<_>>(),
            vec![vec![1, 4], vec![1, 9]],
            "tied a.k rows order by the full (a, b) baseline"
        );
    }

    /// Runs `order_and_limit` with a single candidate and the given ORDER BY
    /// expression, returning the rejection error string.
    fn reject_err(expr: OrderByExpr) -> String {
        let mut c = vec![cand(1, serde_json::json!({"a": {"name": "x"}}))];
        let ob = vec![OrderByItem {
            expr,
            descending: false,
        }];
        order_and_limit(&mut c, Some(&ob), None).expect_err("test: must be rejected")
    }

    #[test]
    fn test_unsupported_arithmetic_rejected() {
        use velesdb_core::velesql::ArithmeticExpr;
        let err = reject_err(OrderByExpr::Arithmetic(ArithmeticExpr::Variable(
            "year".to_string(),
        )));
        assert!(err.contains("arithmetic"), "got: {err}");
    }

    #[test]
    fn test_similarity_bare_rejected() {
        // WASM MATCH materializes no scores, so ORDER BY similarity() is rejected
        // (consistent with similarity(field,$v)/arithmetic) rather than a silent
        // no-op that returns anchor-order masquerading as a relevance ranking.
        let err = reject_err(OrderByExpr::SimilarityBare);
        assert!(err.contains("similarity()"), "got: {err}");
    }

    #[test]
    fn test_bare_field_rejected() {
        let err = reject_err(OrderByExpr::Field("name".to_string()));
        assert!(err.contains("not supported in WASM"), "got: {err}");
    }
}
