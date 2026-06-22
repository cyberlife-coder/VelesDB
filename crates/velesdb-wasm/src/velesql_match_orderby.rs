//! MATCH `RETURN ... ORDER BY` sorting for the WASM executor.
//!
//! Mirrors core's `order_match_results` semantics
//! (`match_exec/order_by.rs` + `similarity.rs`): a deterministic `node_id`
//! tie-break baseline, then each ORDER BY key applied as a STABLE sort
//! least-significant-first (`.rev()`), so multi-key ordering is correct and
//! ties resolve by anchor `node_id`. Sorting happens BEFORE the LIMIT
//! truncation (the caller collects the full candidate set first).
//!
//! Supported keys (the forms WASM can evaluate exactly from in-memory graph
//! rows): zero-arg `similarity()` (the row score), `depth`, and an
//! `alias.property` path. `similarity(field, $v)`, arithmetic, and aggregate
//! expressions require node-vector / score-context evaluation the WASM MATCH
//! path does not materialize, so they are rejected with a clear error rather
//! than silently ignored.

use std::cmp::Ordering;

use velesdb_core::velesql::{OrderByExpr, OrderByItem};

use crate::velesql_result::QueryResultRow;

/// A MATCH result row paired with the data needed to order it.
pub(crate) struct MatchCandidate {
    /// Anchor node id — the deterministic tie-break baseline (mirrors core's
    /// `sort_unstable_by_key(node_id)` before the per-key sorts).
    pub anchor: u64,
    /// Per-row relevance score. The WASM MATCH path performs no vector
    /// scoring, so this is `0.0`; zero-arg `similarity()` still orders by it
    /// (a defined, stable ordering — same as core when scores are absent).
    pub score: f32,
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
/// Returns an error for unsupported ORDER BY expression forms
/// (`similarity(field, $v)`, arithmetic, aggregate).
pub(crate) fn order_and_limit(
    candidates: &mut Vec<MatchCandidate>,
    order_by: Option<&[OrderByItem]>,
    limit: Option<u64>,
) -> Result<(), String> {
    if let Some(items) = order_by {
        // Deterministic baseline: anchor node_id is unique per starting node,
        // giving the stable per-key sorts below a defined tie-break.
        candidates.sort_by_key(|a| a.anchor);
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
        OrderByExpr::SimilarityBare => {
            sort_stable(candidates, descending, |a, b| {
                compare_scores(a.score, b.score)
            });
            Ok(())
        }
        OrderByExpr::Field(f) if f == "depth" => {
            // WASM patterns have a fixed depth per query, so this is a defined
            // no-op within a single pattern — kept for parity, not silently
            // dropped.
            Ok(())
        }
        OrderByExpr::Field(f) => sort_by_property(candidates, f, descending),
        OrderByExpr::Similarity(_) => Err(
            "MATCH ORDER BY similarity(field, $v) is not supported in WASM \
             (use similarity(), depth, or alias.property)"
                .to_string(),
        ),
        OrderByExpr::Arithmetic(_) => Err(
            "MATCH ORDER BY arithmetic expressions are not supported in WASM \
             (use similarity(), depth, or alias.property)"
                .to_string(),
        ),
        _ => Err(
            "MATCH ORDER BY aggregate expression is not supported in WASM \
             (use similarity(), depth, or alias.property)"
                .to_string(),
        ),
    }
}

/// Sorts by an `alias.property` JSON path (dot-nested), nulls last in ASC.
fn sort_by_property(
    candidates: &mut [MatchCandidate],
    path: &str,
    descending: bool,
) -> Result<(), String> {
    if !path.contains('.') {
        return Err(format!(
            "MATCH ORDER BY expression '{path}' is not supported in WASM \
             (use similarity(), depth, or alias.property)"
        ));
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

/// NaN-safe score comparison (NaN sorts last in ASC).
fn compare_scores(a: f32, b: f32) -> Ordering {
    match (a.is_nan(), b.is_nan()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        (false, false) => a.partial_cmp(&b).unwrap_or(Ordering::Equal),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(anchor: u64, score: f32, value: serde_json::Value) -> MatchCandidate {
        MatchCandidate {
            anchor,
            score,
            row: QueryResultRow::synthetic(value.clone()).expect("test: row"),
            value,
        }
    }

    fn anchors(c: &[MatchCandidate]) -> Vec<u64> {
        c.iter().map(|x| x.anchor).collect()
    }

    #[test]
    fn test_order_by_property_asc_then_limit() {
        let mut c = vec![
            cand(1, 0.0, serde_json::json!({"a": {"age": 30}})),
            cand(2, 0.0, serde_json::json!({"a": {"age": 10}})),
            cand(3, 0.0, serde_json::json!({"a": {"age": 20}})),
        ];
        let ob = vec![OrderByItem {
            expr: OrderByExpr::Field("a.age".to_string()),
            descending: false,
        }];
        order_and_limit(&mut c, Some(&ob), Some(2)).expect("test: order");
        // Sorted by age asc (10,20,30) THEN limited to 2 → anchors 2,3.
        assert_eq!(anchors(&c), vec![2, 3]);
    }

    #[test]
    fn test_limit_before_sort_bug_regression() {
        // The bug: limit applied during collection would truncate to the
        // FIRST-seen rows (anchors 1,2) before sorting. Correct behavior keeps
        // the smallest-age rows after sorting.
        let mut c = vec![
            cand(1, 0.0, serde_json::json!({"a": {"age": 99}})),
            cand(2, 0.0, serde_json::json!({"a": {"age": 98}})),
            cand(3, 0.0, serde_json::json!({"a": {"age": 1}})),
        ];
        let ob = vec![OrderByItem {
            expr: OrderByExpr::Field("a.age".to_string()),
            descending: false,
        }];
        order_and_limit(&mut c, Some(&ob), Some(1)).expect("test: order");
        assert_eq!(
            anchors(&c),
            vec![3],
            "smallest age survives, not first-seen"
        );
    }

    #[test]
    fn test_order_by_property_desc() {
        let mut c = vec![
            cand(1, 0.0, serde_json::json!({"a": {"age": 30}})),
            cand(2, 0.0, serde_json::json!({"a": {"age": 10}})),
        ];
        let ob = vec![OrderByItem {
            expr: OrderByExpr::Field("a.age".to_string()),
            descending: true,
        }];
        order_and_limit(&mut c, Some(&ob), None).expect("test: order");
        assert_eq!(anchors(&c), vec![1, 2]);
    }

    #[test]
    fn test_no_order_by_only_limits() {
        let mut c = vec![
            cand(5, 0.0, serde_json::json!({"a": {}})),
            cand(3, 0.0, serde_json::json!({"a": {}})),
            cand(9, 0.0, serde_json::json!({"a": {}})),
        ];
        order_and_limit(&mut c, None, Some(2)).expect("test: order");
        // Traversal order preserved (no sort), just truncated.
        assert_eq!(anchors(&c), vec![5, 3]);
    }

    #[test]
    fn test_tie_break_by_anchor() {
        let mut c = vec![
            cand(3, 0.0, serde_json::json!({"a": {"k": 1}})),
            cand(1, 0.0, serde_json::json!({"a": {"k": 1}})),
            cand(2, 0.0, serde_json::json!({"a": {"k": 1}})),
        ];
        let ob = vec![OrderByItem {
            expr: OrderByExpr::Field("a.k".to_string()),
            descending: false,
        }];
        order_and_limit(&mut c, Some(&ob), None).expect("test: order");
        assert_eq!(
            anchors(&c),
            vec![1, 2, 3],
            "all-equal keys tie-break by anchor"
        );
    }

    #[test]
    fn test_unsupported_arithmetic_rejected() {
        use velesdb_core::velesql::ArithmeticExpr;
        let mut c = vec![cand(1, 0.0, serde_json::json!({"a": {}}))];
        let ob = vec![OrderByItem {
            expr: OrderByExpr::Arithmetic(ArithmeticExpr::Variable("year".to_string())),
            descending: false,
        }];
        let err = order_and_limit(&mut c, Some(&ob), None);
        assert!(err.is_err());
        assert!(err.expect_err("test: err").contains("arithmetic"));
    }

    #[test]
    fn test_bare_field_rejected() {
        let mut c = vec![cand(1, 0.0, serde_json::json!({"a": {"name": "x"}}))];
        let ob = vec![OrderByItem {
            expr: OrderByExpr::Field("name".to_string()),
            descending: false,
        }];
        let err = order_and_limit(&mut c, Some(&ob), None);
        assert!(err.is_err());
    }

    #[test]
    fn test_similarity_bare_orders_by_score() {
        let mut c = vec![
            cand(1, 0.1, serde_json::json!({"a": {}})),
            cand(2, 0.9, serde_json::json!({"a": {}})),
        ];
        let ob = vec![OrderByItem {
            expr: OrderByExpr::SimilarityBare,
            descending: true,
        }];
        order_and_limit(&mut c, Some(&ob), None).expect("test: order");
        assert_eq!(anchors(&c), vec![2, 1]);
    }
}
