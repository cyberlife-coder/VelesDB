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
#[path = "velesql_fusion_tests.rs"]
mod tests;
