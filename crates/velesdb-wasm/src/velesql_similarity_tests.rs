use super::*;
use velesdb_core::velesql::Parser;

use crate::velesql_logic::push_not_inward;
use crate::velesql_value::{parse_params, Params};

fn parse_cond(sql: &str) -> Condition {
    let q = Parser::parse(sql).expect("test: parse");
    q.select.where_clause.expect("test: where")
}

/// `sql`'s WHERE clause, De-Morgan-normalized as the SELECT path reads it.
fn normalized(sql: &str) -> Condition {
    push_not_inward(parse_cond(sql))
}

fn params(json: &str) -> Params {
    parse_params(Some(json)).expect("test: params")
}

/// `v` (dimension 2, cosine): id 1 → [1,0], id 2 → [0,1]; id 1's payload
/// holds a named vector `alt` = [0,1], id 2's holds none.
fn db_with_named_vectors() -> DatabaseInner {
    let mut db = DatabaseInner::new();
    db.create_collection("v", 2, "cosine")
        .expect("test: create");
    for (sql, params) in [
        (
            "INSERT INTO v (id, vector, alt) VALUES (1, $v, $alt)",
            r#"{"v": [1.0, 0.0], "alt": [0.0, 1.0]}"#,
        ),
        (
            "INSERT INTO v (id, vector) VALUES (2, $v)",
            r#"{"v": [0.0, 1.0]}"#,
        ),
    ] {
        crate::velesql_exec::execute(&mut db, sql, Some(params)).expect("test: seed");
    }
    db
}

fn evaluator(db: &DatabaseInner, sql: &str, json: &str) -> Result<SimilarityEvaluator, String> {
    let cond = normalized(sql);
    SimilarityEvaluator::for_condition(db, "v", Some(&cond), &params(json))
        .map_err(|e| e.to_string())
        .map(|e| e.expect("test: the clause has a similarity() leaf"))
}

fn first_leaf(cond: &Condition) -> SimilarityCondition {
    let mut leaves = Vec::new();
    collect_similarity_leaves(cond, &mut leaves);
    leaves[0].clone()
}

#[test]
fn a_clause_without_similarity_needs_no_evaluator() {
    let db = db_with_named_vectors();
    let cond = normalized("SELECT * FROM v WHERE x = 1");
    let eval = SimilarityEvaluator::for_condition(&db, "v", Some(&cond), &params("{}"))
        .expect("test: no error");
    assert!(eval.is_none());
}

/// A query vector the collection cannot score is the SELECT's refusal, raised
/// once before any row is read (`velesql_select::check_select`); the
/// evaluator built after it does not refuse it again, and leaves a row it
/// cannot score UNKNOWN rather than reaching the kernel's length assertion.
#[test]
fn a_query_vector_the_collection_cannot_score_is_refused_once_up_front() {
    let mut db = DatabaseInner::new();
    db.create_metadata_collection("m").expect("test: create");
    let refused = crate::velesql_exec::execute(
        &mut db,
        "SELECT * FROM m WHERE similarity(vector, $q) > 0.5",
        Some(r#"{"q": [1.0, 0.0]}"#),
    )
    .expect_err("test: refused");
    assert!(refused.to_string().contains("[VELES-015]"), "{refused}");

    let mut db = db_with_named_vectors();
    let refused = crate::velesql_exec::execute(
        &mut db,
        "SELECT * FROM v WHERE similarity(vector, $q) > 0.5",
        Some(r#"{"q": [1.0, 0.0, 0.0]}"#),
    )
    .expect_err("test: refused");
    assert!(refused.to_string().contains("[VELES-004]"), "{refused}");
    let eval = evaluator(
        &db,
        "SELECT * FROM v WHERE similarity(vector, $q) > 0.5",
        r#"{"q": [1.0, 0.0, 0.0]}"#,
    )
    .expect("test: no second check");
    assert_eq!((eval.score_at(0), eval.score_at(1)), (None, None));
}

/// Each leaf is scored against its own query vector, so two thresholds over
/// distinct vectors filter independently (core's cascade).
#[test]
fn every_leaf_is_scored_against_its_own_vector() {
    let db = db_with_named_vectors();
    let sql = "SELECT * FROM v WHERE similarity(vector, $a) > 0.5 AND similarity(vector, $b) < 0.5";
    let eval = evaluator(&db, sql, r#"{"a": [1.0, 0.0], "b": [0.0, 1.0]}"#).expect("test: eval");
    let cond = normalized(sql);
    let Condition::And(a, b) = &cond else {
        panic!("test: AND expected, got {cond:?}");
    };
    let (Condition::Similarity(a), Condition::Similarity(b)) = (a.as_ref(), b.as_ref()) else {
        panic!("test: two similarity leaves expected");
    };
    assert_eq!(
        (eval.truth(a, 0), eval.truth(b, 0)),
        (Some(true), Some(true))
    );
    assert_eq!(
        (eval.truth(a, 1), eval.truth(b, 1)),
        (Some(false), Some(false))
    );
    assert_eq!(eval.score_at(0), Some(1.0), "the first leaf's score");
}

/// `similarity(alt, $q)` scores the payload's `alt` array; a row without
/// one makes the leaf UNKNOWN, under NOT too.
#[test]
fn a_named_vector_is_read_from_the_payload() {
    let db = db_with_named_vectors();
    let sql = "SELECT * FROM v WHERE similarity(alt, $q) > 0.5";
    let eval = evaluator(&db, sql, r#"{"q": [0.0, 1.0]}"#).expect("test: eval");
    let leaf = first_leaf(&normalized(sql));
    assert_eq!(eval.truth(&leaf, 0), Some(true));
    assert_eq!(eval.truth(&leaf, 1), None, "no `alt`: UNKNOWN");
    assert_eq!(eval.score_at(1), None);

    let negated = "SELECT * FROM v WHERE NOT similarity(alt, $q) > 0.5";
    let leaf = first_leaf(&normalized(negated));
    assert_eq!(eval.truth(&leaf, 1), None, "NOT UNKNOWN is UNKNOWN");
}

/// A named vector of another dimension cannot be scored: UNKNOWN, not a
/// fabricated score.
#[test]
fn a_named_vector_of_another_dimension_is_unknown() {
    let db = db_with_named_vectors();
    let sql = "SELECT * FROM v WHERE similarity(alt, $q) > 0.5";
    let eval = evaluator(&db, sql, r#"{"q": [0.0, 1.0, 0.0]}"#).expect("test: eval");
    assert_eq!(eval.truth(&first_leaf(&normalized(sql)), 0), None);
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
    let c = normalized("SELECT * FROM t WHERE similarity(vector, $q) > 0.5 OR x = 1");
    assert!(
        strip_similarity(Some(&c)).is_none(),
        "OR(stripped, x) must collapse to None (true OR x = true)"
    );
}

#[test]
fn test_strip_similarity_keeps_other_predicates() {
    let c = normalized("SELECT * FROM t WHERE similarity(vector, $q) > 0.8 AND x = 1");
    let stripped = strip_similarity(Some(&c)).expect("test: stripped");
    assert!(matches!(stripped, Condition::Comparison(_)), "{stripped:?}");
}

#[test]
fn test_strip_similarity_returns_none_when_only_pred() {
    let c = normalized("SELECT * FROM t WHERE similarity(vector, $q) > 0.8");
    assert!(strip_similarity(Some(&c)).is_none());
}

#[test]
fn test_strip_similarity_removes_not_wrapped_predicate() {
    // Normalized first, the NOT-wrapped similarity is a bare (flipped)
    // leaf, so stripping leaves nothing behind.
    let c = normalized("SELECT * FROM t WHERE NOT similarity(vector, $q) > 0.5");
    assert!(strip_similarity(Some(&c)).is_none());
}

#[test]
fn test_strip_similarity_under_not_with_conjunction_keeps_residual() {
    let c = normalized("SELECT * FROM t WHERE NOT similarity(vector, $q) > 0.5 AND x = 1");
    let stripped = strip_similarity(Some(&c)).expect("test: residual kept");
    assert!(matches!(stripped, Condition::Comparison(_)), "{stripped:?}");
}

#[test]
fn not_flips_the_similarity_operator() {
    let leaf = first_leaf(&normalized(
        "SELECT * FROM t WHERE NOT similarity(vector, $q) > 0.5",
    ));
    assert_eq!(leaf.operator, CompareOp::Lte);
    let leaf = first_leaf(&normalized(
        "SELECT * FROM t WHERE similarity(vector, $q) >= 0.8",
    ));
    assert_eq!(leaf.operator, CompareOp::Gte, "no NOT: no flip");
}
