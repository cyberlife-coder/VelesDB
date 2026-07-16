//! Unit tests for lexical relevance scoring.

use super::*;

fn score(query: &str, content: &str) -> f32 {
    lexical_relevance(&terms(query), content)
}

#[test]
fn test_lexical_relevance_full_overlap_scores_one() {
    let score = score("deploy pipeline", "the deploy pipeline is green");
    assert!((score - 1.0).abs() < f32::EPSILON);
}

#[test]
fn test_lexical_relevance_no_overlap_scores_zero() {
    let score = score("deploy pipeline", "unrelated prose entirely");
    assert!(score.abs() < f32::EPSILON);
}

#[test]
fn test_lexical_relevance_partial_overlap_is_fractional() {
    let score = score("deploy pipeline", "the pipeline is green");
    assert!((score - 0.5).abs() < f32::EPSILON);
}

#[test]
fn test_lexical_relevance_is_case_insensitive() {
    assert!((score("DEPLOY", "we deploy nightly") - 1.0).abs() < f32::EPSILON);
}

#[test]
fn test_lexical_relevance_empty_query_scores_zero() {
    assert!(score("", "anything").abs() < f32::EPSILON);
    assert!(score("!!!", "anything").abs() < f32::EPSILON);
}

#[test]
fn test_lexical_relevance_stays_in_unit_interval() {
    let score = score("a b c d", "a a a a b b c c d d");
    assert!((0.0..=1.0).contains(&score));
}

fn recollection(id: u64, content: &str) -> crate::model::Recollection {
    crate::model::Recollection {
        id,
        score: 0.0,
        content: content.to_owned(),
        metadata: None,
    }
}

#[test]
fn test_deterministic_reranker_orders_by_lexical_overlap() {
    let candidates = vec![
        recollection(1, "the cat sat on the mat"),
        recollection(2, "deploy pipeline runs clippy"),
    ];
    let ranked = crate::rerank::Reranker::rerank(
        &DeterministicReranker,
        "deploy pipeline clippy",
        candidates,
    )
    .expect("rerank");
    assert_eq!(ranked[0].id, 2);
}

#[test]
fn test_deterministic_reranker_never_drops_or_invents_ids() {
    let candidates = vec![
        recollection(1, "a"),
        recollection(2, "b"),
        recollection(3, "c"),
    ];
    let ranked = crate::rerank::Reranker::rerank(&DeterministicReranker, "unrelated", candidates)
        .expect("rerank");
    let mut ids: Vec<u64> = ranked.iter().map(|r| r.id).collect();
    ids.sort_unstable();
    assert_eq!(ids, vec![1, 2, 3]);
}

#[test]
fn test_deterministic_reranker_ties_keep_original_order() {
    let candidates = vec![recollection(7, "same words"), recollection(8, "same words")];
    let ranked = crate::rerank::Reranker::rerank(&DeterministicReranker, "same", candidates)
        .expect("rerank");
    assert_eq!(ranked[0].id, 7, "equal scores must keep the incoming order");
}

#[test]
fn test_deterministic_reranker_is_deterministic() {
    let make = || vec![recollection(1, "alpha beta"), recollection(2, "beta gamma")];
    let first =
        crate::rerank::Reranker::rerank(&DeterministicReranker, "beta", make()).expect("rerank");
    let second =
        crate::rerank::Reranker::rerank(&DeterministicReranker, "beta", make()).expect("rerank");
    assert_eq!(
        first.iter().map(|r| r.id).collect::<Vec<_>>(),
        second.iter().map(|r| r.id).collect::<Vec<_>>()
    );
}
