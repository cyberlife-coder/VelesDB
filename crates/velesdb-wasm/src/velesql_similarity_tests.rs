use super::*;
use velesdb_core::velesql::Parser;

fn parse_cond(sql: &str) -> Condition {
    let q = Parser::parse(sql).expect("test: parse");
    q.select.where_clause.expect("test: where")
}

#[test]
fn test_find_similarity_returns_predicate() {
    let c = parse_cond("SELECT * FROM t WHERE similarity(vector, $q) > 0.8");
    assert!(find_similarity(Some(&c)).is_some());
}

#[test]
fn test_find_similarity_returns_none_when_absent() {
    let c = parse_cond("SELECT * FROM t WHERE x = 1");
    assert!(find_similarity(Some(&c)).is_none());
}

#[test]
fn test_strip_similarity_keeps_other_predicates() {
    let c = parse_cond("SELECT * FROM t WHERE similarity(vector, $q) > 0.8 AND x = 1");
    let stripped = strip_similarity(Some(&c)).expect("test: stripped");
    assert!(find_similarity(Some(&stripped)).is_none());
}

#[test]
fn test_strip_similarity_returns_none_when_only_pred() {
    let c = parse_cond("SELECT * FROM t WHERE similarity(vector, $q) > 0.8");
    assert!(strip_similarity(Some(&c)).is_none());
}

#[test]
fn test_evaluator_rejects_metadata_only_collection() {
    let mut db = DatabaseInner::new();
    db.create_metadata_collection("t").expect("test: create");
    let c = parse_cond("SELECT * FROM t WHERE similarity(vector, $q) > 0.5");
    let sim = find_similarity(Some(&c)).expect("test: has sim");
    let params =
        crate::velesql_value::parse_params(Some(r#"{"q": [1.0, 0.0]}"#)).expect("test: params");
    let err = SimilarityEvaluator::new(&db, "t", &sim, &params);
    assert!(err.is_err());
    assert!(err.expect_err("test: err").contains("metadata-only"));
}

#[test]
fn test_evaluator_rejects_dim_mismatch() {
    let mut db = DatabaseInner::new();
    db.create_collection("v", 4, "cosine")
        .expect("test: create");
    let c = parse_cond("SELECT * FROM v WHERE similarity(vector, $q) > 0.5");
    let sim = find_similarity(Some(&c)).expect("test: has sim");
    let params =
        crate::velesql_value::parse_params(Some(r#"{"q": [1.0, 0.0]}"#)).expect("test: params");
    let err = SimilarityEvaluator::new(&db, "v", &sim, &params);
    assert!(err.is_err());
    assert!(err.expect_err("test: err").contains("dimension mismatch"));
}

// --- NOT similarity rewriting (finding E) ---------------------------------

#[test]
fn test_flip_similarity_op_is_logical_complement() {
    assert_eq!(flip_similarity_op(CompareOp::Gt), CompareOp::Lte);
    assert_eq!(flip_similarity_op(CompareOp::Gte), CompareOp::Lt);
    assert_eq!(flip_similarity_op(CompareOp::Lt), CompareOp::Gte);
    assert_eq!(flip_similarity_op(CompareOp::Lte), CompareOp::Gt);
    assert_eq!(flip_similarity_op(CompareOp::Eq), CompareOp::NotEq);
    assert_eq!(flip_similarity_op(CompareOp::NotEq), CompareOp::Eq);
}

#[test]
fn test_find_similarity_flips_op_under_not() {
    let c = parse_cond("SELECT * FROM t WHERE NOT similarity(vector, $q) > 0.5");
    let sim = find_similarity(Some(&c)).expect("test: flipped similarity");
    // `NOT sim > 0.5` surfaces as `sim <= 0.5`.
    assert_eq!(sim.operator, CompareOp::Lte);
    assert!((sim.threshold - 0.5).abs() < f64::EPSILON);
}

#[test]
fn test_strip_similarity_removes_not_wrapped_predicate() {
    // After strip, the similarity subtree (including its NOT wrapper)
    // must be gone from the residual. Without the normalization, the
    // pre-fix implementation left the raw (un-flipped) similarity
    // behind, which velesql_where::matches would reject.
    let c = parse_cond("SELECT * FROM t WHERE NOT similarity(vector, $q) > 0.5");
    let stripped = strip_similarity(Some(&c));
    assert!(
        stripped.is_none(),
        "NOT similarity predicate must be fully removed from residual, got {stripped:?}"
    );
}

#[test]
fn test_strip_similarity_under_not_with_conjunction_keeps_residual() {
    // `NOT sim > 0.5 AND x = 1` → after strip, only `x = 1` remains,
    // since the NOT-similarity is surfaced via find_similarity with a
    // flipped op.
    let c = parse_cond("SELECT * FROM t WHERE NOT similarity(vector, $q) > 0.5 AND x = 1");
    let stripped = strip_similarity(Some(&c)).expect("test: residual kept");
    assert!(find_similarity(Some(&stripped)).is_none());
}

#[test]
fn test_find_similarity_preserves_op_without_not() {
    // Non-regression: no NOT wrapper means the operator is returned
    // as-is (no accidental flip).
    let c = parse_cond("SELECT * FROM t WHERE similarity(vector, $q) >= 0.8");
    let sim = find_similarity(Some(&c)).expect("test: plain similarity");
    assert_eq!(sim.operator, CompareOp::Gte);
}

// --- combine_after_strip: finding G ---------------------------------------

fn leaf(op: CompareOp, v: i64) -> Condition {
    use velesdb_core::velesql::{Comparison, Value};
    Condition::Comparison(Comparison {
        column: "x".into(),
        operator: op,
        value: Value::Integer(v),
    })
}

#[test]
fn test_combine_and_both_some_rebuilds_and_node() {
    let a = leaf(CompareOp::Eq, 1);
    let b = leaf(CompareOp::Eq, 2);
    let out = combine_after_strip(Some(a), Some(b), LogicalOp::And);
    assert!(matches!(out, Some(Condition::And(_, _))));
}

#[test]
fn test_combine_and_one_none_returns_surviving_side() {
    let a = leaf(CompareOp::Eq, 1);
    let out = combine_after_strip(Some(a.clone()), None, LogicalOp::And);
    assert_eq!(out, Some(a.clone()));
    let out = combine_after_strip(None, Some(a.clone()), LogicalOp::And);
    assert_eq!(out, Some(a));
}

#[test]
fn test_combine_and_both_none_is_none() {
    assert!(combine_after_strip(None, None, LogicalOp::And).is_none());
}

#[test]
fn test_combine_or_both_some_rebuilds_or_node() {
    let a = leaf(CompareOp::Eq, 1);
    let b = leaf(CompareOp::Eq, 2);
    let out = combine_after_strip(Some(a), Some(b), LogicalOp::Or);
    assert!(matches!(out, Some(Condition::Or(_, _))));
}

#[test]
fn test_combine_or_one_none_collapses_to_none() {
    // The key fix for finding G: `true OR x = true`, so the residual
    // post-filter is None — NOT the surviving side.
    let a = leaf(CompareOp::Eq, 1);
    assert!(combine_after_strip(Some(a.clone()), None, LogicalOp::Or).is_none());
    assert!(combine_after_strip(None, Some(a), LogicalOp::Or).is_none());
}

#[test]
fn test_combine_or_both_none_is_none() {
    assert!(combine_after_strip(None, None, LogicalOp::Or).is_none());
}

#[test]
fn test_strip_similarity_or_predicate_collapses_to_none() {
    // BDD-style regression of finding G at the strip layer: a
    // `similarity() OR x = 1` query has `None` residual after
    // stripping the similarity leaf — the OR is trivially satisfied.
    let c = parse_cond("SELECT * FROM t WHERE similarity(vector, $q) > 0.5 OR x = 1");
    assert!(
        strip_similarity(Some(&c)).is_none(),
        "OR(stripped, x) must collapse to None (true OR x = true)"
    );
}

#[test]
fn test_strip_similarity_and_predicate_keeps_predicate() {
    // Non-regression: `similarity() AND x = 1` still strips to `x = 1`.
    let c = parse_cond("SELECT * FROM t WHERE similarity(vector, $q) > 0.5 AND x = 1");
    let residual = strip_similarity(Some(&c)).expect("test: residual");
    assert!(find_similarity(Some(&residual)).is_none());
}

// --- assert_single_similarity_vector: finding H ---------------------------

#[test]
fn test_assert_single_sim_vec_accepts_none() {
    assert!(assert_single_similarity_vector(None).is_ok());
}

#[test]
fn test_assert_single_sim_vec_accepts_no_similarity() {
    let c = parse_cond("SELECT * FROM t WHERE x = 1");
    assert!(assert_single_similarity_vector(Some(&c)).is_ok());
}

#[test]
fn test_assert_single_sim_vec_accepts_single_predicate() {
    let c = parse_cond("SELECT * FROM t WHERE similarity(vector, $q) > 0.5");
    assert!(assert_single_similarity_vector(Some(&c)).is_ok());
}

#[test]
fn test_assert_single_sim_vec_accepts_same_param_twice() {
    let c = parse_cond(
        "SELECT * FROM t WHERE similarity(vector, $q) > 0.5 AND similarity(vector, $q) < 0.9",
    );
    assert!(assert_single_similarity_vector(Some(&c)).is_ok());
}

#[test]
fn test_assert_single_sim_vec_rejects_different_params_and() {
    let c = parse_cond(
        "SELECT * FROM t WHERE similarity(vector, $a) > 0.5 AND similarity(vector, $b) > 0.3",
    );
    let err = assert_single_similarity_vector(Some(&c));
    assert!(err.is_err());
    assert!(
        err.expect_err("test: err")
            .contains("Multiple similarity()"),
        "error must name the feature"
    );
}

#[test]
fn test_assert_single_sim_vec_rejects_different_params_or() {
    let c = parse_cond(
        "SELECT * FROM t WHERE similarity(vector, $a) > 0.5 OR similarity(vector, $b) > 0.3",
    );
    assert!(assert_single_similarity_vector(Some(&c)).is_err());
}

#[test]
fn test_assert_single_sim_vec_rejects_param_vs_literal() {
    // A param vector and a literal vector are distinct VectorExprs
    // even if the runtime-bound value would match — identity is at
    // the AST level.
    let c = parse_cond(
        "SELECT * FROM t WHERE similarity(vector, $q) > 0.5 AND similarity(vector, [1.0, 0.0]) > 0.3",
    );
    assert!(assert_single_similarity_vector(Some(&c)).is_err());
}

#[test]
fn test_assert_single_sim_vec_accepts_same_literal_twice() {
    let c = parse_cond(
        "SELECT * FROM t WHERE similarity(vector, [1.0, 0.0]) > 0.5 AND similarity(vector, [1.0, 0.0]) < 0.9",
    );
    assert!(assert_single_similarity_vector(Some(&c)).is_ok());
}

#[test]
fn test_assert_single_sim_vec_walks_under_not_compound() {
    // A compound `NOT (sim_a AND sim_b)` must still be caught.
    // After De-Morgan normalization it becomes `sim_a' OR sim_b'` with
    // flipped ops — the two distinct vectors are still present.
    let raw = parse_cond(
        "SELECT * FROM t WHERE NOT (similarity(vector, $a) > 0.5 AND similarity(vector, $b) > 0.3)",
    );
    let normalized = push_not_inward(raw);
    assert!(assert_single_similarity_vector(Some(&normalized)).is_err());
}
