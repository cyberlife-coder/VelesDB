use super::*;
use crate::velesql::{CompareOp, Comparison, SimilarityCondition, Value, VectorExpr};

fn make_sim_condition() -> Condition {
    Condition::Similarity(SimilarityCondition {
        field: "doc.embedding".to_string(),
        vector: VectorExpr::Parameter("query".to_string()),
        operator: CompareOp::Gt,
        threshold: 0.8,
    })
}

fn make_comparison_condition() -> Condition {
    Condition::Comparison(Comparison {
        column: "category".to_string(),
        operator: CompareOp::Eq,
        value: Value::String("tech".to_string()),
    })
}

#[test]
fn test_extract_similarity_condition_direct() {
    let sim = make_sim_condition();
    let extracted = extract_similarity_condition(Some(&sim)).expect("should find similarity");
    assert_eq!(extracted.field, "doc.embedding");
    assert!((extracted.threshold - 0.8).abs() < f64::EPSILON);
}

#[test]
fn test_extract_similarity_condition_nested_and() {
    let cond = Condition::And(
        Box::new(make_comparison_condition()),
        Box::new(make_sim_condition()),
    );
    let extracted = extract_similarity_condition(Some(&cond)).expect("should find in AND");
    assert_eq!(extracted.field, "doc.embedding");
}

#[test]
fn test_extract_similarity_condition_missing() {
    let cond = make_comparison_condition();
    let result = extract_similarity_condition(Some(&cond));
    assert!(result.is_err(), "should fail when no similarity present");
}

#[test]
fn test_extract_similarity_condition_none_where() {
    let result = extract_similarity_condition(None);
    assert!(result.is_err(), "should fail on None where clause");
}

#[test]
fn test_strip_similarity_removes_direct() {
    let sim = make_sim_condition();
    let stripped = strip_similarity_from_where(Some(&sim));
    assert!(stripped.is_none(), "bare similarity should strip to None");
}

#[test]
fn test_strip_similarity_keeps_other_in_and() {
    let cmp = make_comparison_condition();
    let sim = make_sim_condition();
    let cond = Condition::And(Box::new(sim), Box::new(cmp.clone()));
    let stripped = strip_similarity_from_where(Some(&cond));
    assert!(stripped.is_some(), "AND(sim, cmp) should keep cmp");
    assert!(
        matches!(stripped, Some(Condition::Comparison(_))),
        "result should be the comparison"
    );
}

#[test]
fn test_strip_similarity_drops_entire_or() {
    let cmp = make_comparison_condition();
    let sim = make_sim_condition();
    let cond = Condition::Or(Box::new(cmp.clone()), Box::new(sim));
    let stripped = strip_similarity_from_where(Some(&cond));
    assert!(
        stripped.is_none(),
        "OR(cmp, sim) should be None — the similarity branch satisfies the entire OR"
    );
}

#[test]
fn test_strip_similarity_preserves_or_without_sim() {
    let cmp1 = make_comparison_condition();
    let cmp2 = Condition::Comparison(Comparison {
        column: "price".to_string(),
        operator: CompareOp::Gt,
        value: Value::Float(42.0),
    });
    let cond = Condition::Or(Box::new(cmp1), Box::new(cmp2));
    let stripped = strip_similarity_from_where(Some(&cond));
    assert!(
        matches!(stripped, Some(Condition::Or(..))),
        "OR(cmp1, cmp2) with no similarity should preserve both branches"
    );
}

#[test]
fn test_strip_similarity_preserves_non_sim_tree() {
    let cmp = make_comparison_condition();
    let stripped = strip_similarity_from_where(Some(&cmp));
    assert!(
        matches!(stripped, Some(Condition::Comparison(_))),
        "non-similarity condition should pass through"
    );
}

#[test]
fn test_passes_threshold_higher_is_better() {
    assert!(passes_threshold(0.9, 0.8, true));
    assert!(passes_threshold(0.8, 0.8, true));
    assert!(!passes_threshold(0.7, 0.8, true));
}

#[test]
fn test_passes_threshold_lower_is_better() {
    assert!(passes_threshold(0.3, 0.5, false));
    assert!(passes_threshold(0.5, 0.5, false));
    assert!(!passes_threshold(0.7, 0.5, false));
}

// Regression test: Devin review — NOT(Similarity) must be preserved.
// Stripping NOT(sim) inverts query semantics: "reject high-similarity"
// becomes "no filter at all".

#[test]
fn test_strip_sim_preserves_not_similarity() {
    let sim = make_sim_condition();
    let cond = Condition::Not(Box::new(sim));
    let stripped = strip_similarity_from_where(Some(&cond));
    assert!(
        matches!(stripped, Some(Condition::Not(_))),
        "NOT(Similarity) must be preserved — it is a meaningful residual filter"
    );
}

#[test]
fn test_strip_sim_preserves_not_non_sim() {
    let cmp = make_comparison_condition();
    let cond = Condition::Not(Box::new(cmp));
    let stripped = strip_similarity_from_where(Some(&cond));
    assert!(
        matches!(stripped, Some(Condition::Not(_))),
        "NOT(comparison) should always be preserved"
    );
}
