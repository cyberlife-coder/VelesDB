//! Unit tests for the fusion ranking layer.

use super::*;
use crate::model::{FusionOptions, Recollection};

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

#[test]
fn test_fuse_negative_vector_score_never_beats_a_stronger_positive_one() {
    // Regression: Cosine scores range over [-1, 1], so a negative
    // vector_score is a normal, in-range "dissimilar" result, not an error
    // state. The old normalization divided it by a positive epsilon-floored
    // divisor, producing an unbounded negative fused score (observed
    // ~-2.3e14 for a realistic Cosine value) — nonsensical in scale, but
    // more importantly the fix must not let a *weak positive* signal lose to
    // a *negative* one just because both get divided by the same max_score.
    let pool = vec![candidate(1, 0.05, 0.0), candidate(2, -0.9, 0.0)];
    let ranked = fuse(pool, &[], 2, 0.15);
    assert_eq!(
        ranked.iter().map(|r| r.id).collect::<Vec<_>>(),
        vec![1, 2],
        "a positive (even weak) vector score must outrank a negative one"
    );
}

#[test]
fn test_fuse_all_negative_pool_ties_at_a_neutral_baseline_not_an_inverted_extreme() {
    // A negative vector_score is floored to 0 before normalising — the same
    // neutral baseline a graph-only candidate (vector_score = 0.0) already
    // carries — rather than propagating its sign through the division. Two
    // different negative scores therefore both land at fused_score = 0 (a
    // stable sort keeps their relative pool order), never at astronomically
    // different, sign-flipped magnitudes.
    let pool = vec![candidate(1, -0.05, 0.0), candidate(2, -0.9, 0.0)];
    let reached = vec![candidate(3, 0.0, 1.0)];
    let ranked = fuse(pool, &reached, 3, 0.15);
    assert_eq!(
        ranked.iter().map(|r| r.id).collect::<Vec<_>>(),
        vec![3, 1, 2],
        "the graph-reached candidate outranks the tied, neutral-baseline negative pool (stable order preserved)"
    );
}

#[test]
fn test_non_finite_graph_boost_collapses_fusion_without_sanitizing() {
    // Guards the reason FusionOptions::sanitized exists: a non-finite boost
    // makes `graph_boost · weight` NaN for EVERY candidate (even a pool-only
    // one, since NaN·0.0 == NaN), so total_cmp sees all scores equal, the sort
    // is a no-op, and the graph-reached fact — appended after the pool — is
    // truncated away. A strong pool hit (id 1) plus a weak one (id 2) that the
    // default boost is enough to outrank; the graph fact (id 3) should take the
    // second slot at k=2.
    let build = || {
        (
            vec![candidate(1, 0.5, 0.0), candidate(2, 0.01, 0.0)],
            vec![candidate(3, 0.0, 1.0)],
        )
    };

    let (pool, reached) = build();
    let ranked = fuse(pool, &reached, 2, f64::NAN);
    assert!(
        !ranked.iter().any(|r| r.id == 3),
        "with a raw NaN boost the graph fact is silently dropped — this is the failure sanitized() prevents"
    );

    // Sanitizing the boost restores correct fusion: the default 0.15 outranks
    // the weak pool hit, so the graph fact takes the second slot.
    let boost = FusionOptions {
        graph_boost: f64::NAN,
        ..FusionOptions::default()
    }
    .sanitized()
    .graph_boost;
    let (pool, reached) = build();
    let ranked = fuse(pool, &reached, 2, boost);
    assert!(
        ranked.iter().any(|r| r.id == 3),
        "after sanitizing the boost, the graph fact ranks in again"
    );
}

#[test]
#[allow(clippy::float_cmp)] // asserting exact propagation of literal boosts, no arithmetic
fn test_fusion_options_sanitized_only_touches_non_finite_boost() {
    let default_boost = FusionOptions::default().graph_boost;
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let opts = FusionOptions {
            graph_boost: bad,
            ..FusionOptions::default()
        }
        .sanitized();
        assert_eq!(opts.graph_boost, default_boost);
    }
    // A finite (even negative) boost is a valid demote-the-graph choice, left as is.
    let kept = FusionOptions {
        graph_boost: -0.5,
        ..FusionOptions::default()
    }
    .sanitized();
    assert_eq!(kept.graph_boost, -0.5);
}

#[test]
#[allow(clippy::float_cmp)] // asserting exact propagation of literal boosts, no arithmetic
fn test_fusion_options_from_knobs_defaults_and_clamps_hops() {
    let d = FusionOptions::default();
    let none = FusionOptions::from_knobs(None, None, None);
    assert_eq!(none.hops, d.hops);
    assert_eq!(none.graph_boost, d.graph_boost);
    assert_eq!(none.pool, d.pool);

    let clamped = FusionOptions::from_knobs(Some(usize::MAX), Some(0.42), Some(usize::MAX));
    assert_eq!(clamped.hops, crate::limits::clamp_hops(usize::MAX));
    assert_eq!(clamped.graph_boost, 0.42);
    assert_eq!(
        clamped.pool,
        Some(crate::limits::clamp_recall_limit(usize::MAX))
    );
}
