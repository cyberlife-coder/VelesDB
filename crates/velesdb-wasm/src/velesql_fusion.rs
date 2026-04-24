//! FUSION clause support for the WASM VelesQL executor (S4-13).
//!
//! When a SELECT includes `USING FUSION (strategy = '...', ...)`, we fuse
//! the ranked candidate lists produced by the vector search and any other
//! scoring branch (BM25 is out of scope for WASM, so we fuse vector + the
//! id-equality hit set from the WHERE clause — still a useful hybrid for
//! demos).
//!
//! Supports the two most common strategies used in the wild:
//! - `rrf` (Reciprocal Rank Fusion, k default 60)
//! - `weighted` (per-branch min-max normalized score)
//!
//! Unknown strategies fall back to `rrf` (with a logged warning in native
//! tests) so a demo SQL never errors mid-query on a misspelled strategy.

use velesdb_core::fusion::FusionStrategy;
use velesdb_core::velesql::{FusionClause, FusionStrategyType};

/// Represents a single ranked branch: `(id, score)` pairs sorted descending.
pub(crate) type RankedBranch = Vec<(u64, f32)>;

/// Applies the clause's fusion strategy to the given branches.
///
/// Falls back to RRF with k=60 on validation errors (weight-sum mismatch,
/// negative weights) so the caller always gets a useful ranking.
pub(crate) fn apply(clause: &FusionClause, branches: Vec<RankedBranch>) -> Vec<(u64, f32)> {
    let strategy = build_strategy(clause);
    strategy.fuse(branches).unwrap_or_default()
}

/// Maps the AST clause onto a concrete [`FusionStrategy`].
fn build_strategy(clause: &FusionClause) -> FusionStrategy {
    match clause.strategy {
        FusionStrategyType::Rrf => FusionStrategy::RRF {
            k: clause.k.unwrap_or(60),
        },
        FusionStrategyType::Maximum => FusionStrategy::Maximum,
        FusionStrategyType::Average => FusionStrategy::Average,
        FusionStrategyType::Weighted => build_weighted(clause),
        FusionStrategyType::Rsf => build_rsf(clause),
        _ => FusionStrategy::rrf_default(),
    }
}

fn build_weighted(clause: &FusionClause) -> FusionStrategy {
    #[allow(clippy::cast_possible_truncation)]
    let vector_weight = clause.vector_weight.unwrap_or(0.5) as f32;
    #[allow(clippy::cast_possible_truncation)]
    let graph_weight = clause.graph_weight.unwrap_or(0.5) as f32;
    // Weighted fusion uses (avg_weight, max_weight, hit_weight). Map
    // vector_weight -> max_weight (favour top vector hits), and
    // graph_weight -> avg_weight (average over branches). Remainder goes
    // to hit ratio so the weights always sum to 1.0.
    let hit = (1.0 - vector_weight - graph_weight).clamp(0.0, 1.0);
    FusionStrategy::weighted(graph_weight, vector_weight, hit)
        .unwrap_or_else(|_| FusionStrategy::rrf_default())
}

fn build_rsf(clause: &FusionClause) -> FusionStrategy {
    let dense = clause.dense_weight.unwrap_or(0.5);
    let sparse = clause.sparse_weight.unwrap_or(0.5);
    FusionStrategy::relative_score(dense, sparse).unwrap_or_else(|_| FusionStrategy::rrf_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rrf(k: u32) -> FusionClause {
        FusionClause {
            strategy: FusionStrategyType::Rrf,
            k: Some(k),
            vector_weight: None,
            graph_weight: None,
            dense_weight: None,
            sparse_weight: None,
        }
    }

    #[test]
    fn test_apply_rrf_returns_combined_ranking() {
        let b1 = vec![(1, 0.9), (2, 0.8), (3, 0.7)];
        let b2 = vec![(3, 0.95), (2, 0.85), (4, 0.75)];
        let fused = apply(&rrf(60), vec![b1, b2]);
        assert!(!fused.is_empty());
        // Id 2 and 3 appear in both branches and should rank above 1/4.
        let top_two: Vec<u64> = fused.iter().take(2).map(|&(id, _)| id).collect();
        assert!(top_two.contains(&2) || top_two.contains(&3));
    }

    #[test]
    fn test_apply_empty_branches_returns_empty() {
        let fused = apply(&rrf(60), vec![]);
        assert!(fused.is_empty());
    }

    #[test]
    fn test_apply_unknown_strategy_falls_back_to_rrf() {
        // Weighted with invalid weights (both 2.0) fails validation and
        // must fall back to RRF without panicking.
        let bad = FusionClause {
            strategy: FusionStrategyType::Weighted,
            k: None,
            vector_weight: Some(2.0),
            graph_weight: Some(2.0),
            dense_weight: None,
            sparse_weight: None,
        };
        let fused = apply(&bad, vec![vec![(1, 0.5)], vec![(2, 0.7)]]);
        assert!(!fused.is_empty());
    }

    #[test]
    fn test_apply_maximum_picks_top_score_per_id() {
        let clause = FusionClause {
            strategy: FusionStrategyType::Maximum,
            k: None,
            vector_weight: None,
            graph_weight: None,
            dense_weight: None,
            sparse_weight: None,
        };
        let b1 = vec![(1, 0.2), (2, 0.5)];
        let b2 = vec![(1, 0.9), (3, 0.1)];
        let fused = apply(&clause, vec![b1, b2]);
        let id1 = fused
            .iter()
            .find(|(id, _)| *id == 1)
            .expect("test: id 1 present");
        // Maximum should pick the 0.9 from branch 2.
        assert!(id1.1 >= 0.85);
    }
}
