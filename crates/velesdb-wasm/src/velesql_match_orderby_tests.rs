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
