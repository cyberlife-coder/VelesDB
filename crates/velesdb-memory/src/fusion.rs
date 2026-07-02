//! Vector+graph score fusion — the ranking layer behind
//! [`MemoryService::recall_fused`](crate::service::MemoryService::recall_fused).
//!
//! Ported from the LoCoMo benchmark harness (`examples/locomo/eval.rs`), where
//! this exact re-ranking measured a generation-free lift on public multi-hop
//! benchmarks (+6.9pp both-facts recall, HotpotQA 3000-Q; +9.7pp gold-sentence
//! recall, TimeQA) that the shipped `recall()` — pure vector search — never
//! captured, because it never combined the vector and graph facets.

use std::collections::{HashMap, HashSet};

use crate::model::Recollection;

/// Depth of the oversampled vector pool a fused recall re-ranks, and its floor
/// regardless of `k`: deep enough that a graph-promoted fact has room to
/// surface without evicting a genuinely stronger vector hit. Values proven on
/// HotpotQA/TimeQA/LoCoMo.
pub(crate) const POOL_FACTOR: usize = 8;
pub(crate) const POOL_MIN: usize = 64;

/// Oversampled candidate pool depth for a `k`-sized fused recall.
pub(crate) fn pool_size(k: usize) -> usize {
    k.saturating_mul(POOL_FACTOR).max(POOL_MIN)
}

/// A recall candidate carrying its raw vector score and (if graph-reached) a
/// graph promotion weight. Internal fusion currency — distinct from
/// [`Recollection`], the public return shape.
#[derive(Debug, Clone)]
pub(crate) struct Candidate {
    pub recollection: Recollection,
    /// Raw vector similarity, `0.0` for a fact the vector pool never ranked
    /// (it entered only via graph traversal).
    pub vector_score: f64,
    /// Graph promotion weight: `0.0` for a pool-only hit, `> 0.0` for a fact
    /// the graph traversal reached (a flat `1.0`, or an idf-based weight).
    pub graph_weight: f64,
}

/// Re-rank `pool ∪ reached` by `vector_score/max_score + graph_boost·graph_weight`,
/// take the top `k`. A fact both vector-ranked and graph-reached keeps its pool
/// copy (its real vector score, plus the reached weight folded in by
/// [`fused_score`]); a fact the graph reaches but the pool never ranked
/// carries `vector_score = 0.0` and rides on its `graph_weight` alone.
///
/// Equal-budget promotion, not blind eviction: a strong vector fact keeps its
/// place unless a graph-connected fact's boosted score outranks it.
pub(crate) fn fuse(
    pool: Vec<Candidate>,
    reached: &[Candidate],
    k: usize,
    graph_boost: f64,
) -> Vec<Recollection> {
    let weights: HashMap<u64, f64> = reached
        .iter()
        .map(|c| (c.recollection.id, c.graph_weight))
        .collect();
    let max_score = pool
        .iter()
        .map(|c| c.vector_score)
        .fold(f64::MIN, f64::max)
        .max(f64::EPSILON);

    let mut candidates = pool;
    let present: HashSet<u64> = candidates.iter().map(|c| c.recollection.id).collect();
    candidates.extend(
        reached
            .iter()
            .filter(|c| !present.contains(&c.recollection.id))
            .cloned(),
    );

    candidates.sort_by(|a, b| {
        fused_score(b, &weights, max_score, graph_boost).total_cmp(&fused_score(
            a,
            &weights,
            max_score,
            graph_boost,
        ))
    });
    candidates.truncate(k);
    candidates.into_iter().map(|c| c.recollection).collect()
}

/// `max(vector_score, 0)/max_score + graph_boost·graph_weight`. A pure
/// vector hit (`graph_weight` absent from `weights`) keeps its bare
/// normalised similarity.
///
/// The numerator is floored at `0`, not just the divisor: Cosine scores
/// range over `[-1, 1]`, so a negative `vector_score` is a legitimate,
/// in-range "dissimilar" result, not an error state — dividing a negative
/// numerator by an epsilon-floored *positive* divisor would otherwise invert
/// its sign into an unbounded negative score (regression: an all-negative
/// pool scored around `-2.3e14`, dwarfing any `graph_boost` regardless of
/// actual relevance). Flooring the numerator instead means a fact with no
/// positive vector signal contributes `0` — the same neutral baseline a
/// graph-only candidate (`vector_score = 0.0`) already gets — so it can
/// still be promoted by a real graph connection, but by the same bounded
/// margin as any other zero-vector-signal candidate, never by an
/// astronomical, sign-flipped one.
fn fused_score(
    candidate: &Candidate,
    weights: &HashMap<u64, f64>,
    max_score: f64,
    graph_boost: f64,
) -> f64 {
    let weight = weights
        .get(&candidate.recollection.id)
        .copied()
        .unwrap_or(0.0);
    candidate.vector_score.max(0.0) / max_score + graph_boost * weight
}

#[cfg(test)]
#[path = "fusion_tests.rs"]
mod tests;
