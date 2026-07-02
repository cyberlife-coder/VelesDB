//! Unit tests for the fusion ranking layer.

use super::*;
use crate::model::Recollection;

#[allow(clippy::cast_possible_truncation)] // test fixture scores are small, exact literals
fn candidate(id: u64, vector_score: f64, graph_weight: f64) -> Candidate {
    Candidate {
        recollection: Recollection {
            id,
            score: vector_score as f32,
            content: format!("fact-{id}"),
            metadata: None,
        },
        vector_score,
        graph_weight,
    }
}

#[test]
fn test_pool_size_floors_at_pool_min() {
    assert_eq!(pool_size(1), POOL_MIN);
    assert_eq!(pool_size(4), POOL_MIN);
}

#[test]
fn test_pool_size_scales_with_k_above_floor() {
    assert_eq!(pool_size(100), 800);
}

#[test]
fn test_fuse_pure_vector_pool_keeps_score_order() {
    let pool = vec![candidate(1, 0.9, 0.0), candidate(2, 0.5, 0.0)];
    let ranked = fuse(pool, &[], 2, 0.15);
    assert_eq!(ranked.iter().map(|r| r.id).collect::<Vec<_>>(), vec![1, 2]);
}

#[test]
fn test_fuse_promotes_graph_reached_fact_not_in_pool() {
    // Pool has two vector hits, the 2nd clearly weaker; the graph reaches
    // a 3rd fact with a strong weight. It must be able to outrank the
    // weaker pool member, not just get appended.
    let pool = vec![candidate(1, 0.5, 0.0), candidate(2, 0.3, 0.0)];
    let reached = vec![candidate(3, 0.0, 1.0)];
    let ranked = fuse(pool, &reached, 2, 0.8);
    assert_eq!(ranked.len(), 2);
    assert!(
        ranked.iter().any(|r| r.id == 3),
        "graph fact should surface"
    );
    assert!(
        !ranked.iter().any(|r| r.id == 2),
        "weaker vector hit should be displaced"
    );
}

#[test]
fn test_fuse_never_evicts_strong_vector_hit_with_weak_boost() {
    // A small graph_boost must not let a graph-only fact beat a strong,
    // well-ranked vector hit.
    let pool = vec![candidate(1, 0.95, 0.0), candidate(2, 0.5, 0.0)];
    let reached = vec![candidate(3, 0.0, 1.0)];
    let ranked = fuse(pool, &reached, 1, 0.05);
    assert_eq!(ranked.len(), 1);
    assert_eq!(ranked[0].id, 1);
}

#[test]
fn test_fuse_dedups_fact_both_vector_ranked_and_graph_reached() {
    let pool = vec![candidate(1, 0.9, 0.0)];
    let reached = vec![candidate(1, 0.0, 1.0)];
    let ranked = fuse(pool, &reached, 5, 0.15);
    assert_eq!(ranked.len(), 1);
    // The pool copy's real vector score must win, boosted by the reached
    // weight — not the reached copy's placeholder 0.0 vector score.
    assert!(ranked[0].score > 0.0);
}

#[test]
fn test_fuse_truncates_to_k() {
    let pool = vec![
        candidate(1, 0.9, 0.0),
        candidate(2, 0.8, 0.0),
        candidate(3, 0.7, 0.0),
    ];
    let ranked = fuse(pool, &[], 1, 0.15);
    assert_eq!(ranked.len(), 1);
    assert_eq!(ranked[0].id, 1);
}

#[test]
fn test_fuse_empty_pool_and_reached_yields_empty() {
    let ranked = fuse(Vec::new(), &[], 5, 0.15);
    assert!(ranked.is_empty());
}
