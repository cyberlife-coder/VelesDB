//! Unit tests for lexical relevance scoring.

use super::*;

#[test]
fn test_lexical_relevance_full_overlap_scores_one() {
    let score = lexical_relevance("deploy pipeline", "the deploy pipeline is green");
    assert!((score - 1.0).abs() < f32::EPSILON);
}

#[test]
fn test_lexical_relevance_no_overlap_scores_zero() {
    let score = lexical_relevance("deploy pipeline", "unrelated prose entirely");
    assert!(score.abs() < f32::EPSILON);
}

#[test]
fn test_lexical_relevance_partial_overlap_is_fractional() {
    let score = lexical_relevance("deploy pipeline", "the pipeline is green");
    assert!((score - 0.5).abs() < f32::EPSILON);
}

#[test]
fn test_lexical_relevance_is_case_insensitive() {
    assert!((lexical_relevance("DEPLOY", "we deploy nightly") - 1.0).abs() < f32::EPSILON);
}

#[test]
fn test_lexical_relevance_empty_query_scores_zero() {
    assert!(lexical_relevance("", "anything").abs() < f32::EPSILON);
    assert!(lexical_relevance("!!!", "anything").abs() < f32::EPSILON);
}

#[test]
fn test_lexical_relevance_stays_in_unit_interval() {
    let score = lexical_relevance("a b c d", "a a a a b b c c d d");
    assert!((0.0..=1.0).contains(&score));
}
