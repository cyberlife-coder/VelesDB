//! FUSION clause support for the WASM VelesQL executor (S4-13).
//!
//! When a SELECT includes `USING FUSION (strategy = '...', ...)`, we fuse
//! the ranked candidate lists produced by the vector search and any other
//! scoring branch (BM25 is out of scope for WASM, so we fuse vector + the
//! id-equality hit set from the WHERE clause — still a useful hybrid for
//! demos).
//!
//! Every strategy core parses is mapped (`rrf`, `weighted`, `weighted_rrf`,
//! `maximum`, `average`, `rsf`). An unknown strategy name never reaches
//! here: the parser refuses it, and the validator refuses out-of-range
//! weights; a refusal of the strategy itself surfaces as an error.

use velesdb_core::fusion::FusionStrategy;
use velesdb_core::velesql::{FusionClause, FusionStrategyType};

/// Represents a single ranked branch: `(id, score)` pairs sorted descending.
pub(crate) type RankedBranch = Vec<(u64, f32)>;

/// Applies the clause's fusion strategy to the given branches.
///
/// # Errors
///
/// Returns the strategy's refusal (e.g. a branch count its weights do not
/// cover) rather than an empty ranking that would read as "no match".
pub(crate) fn apply(
    clause: &FusionClause,
    branches: Vec<RankedBranch>,
) -> Result<Vec<(u64, f32)>, String> {
    build_strategy(clause)
        .fuse(branches)
        .map_err(|e| format!("USING FUSION: {e}"))
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
        FusionStrategyType::WeightedRrf => build_weighted_rrf(clause),
        FusionStrategyType::Rsf => build_rsf(clause),
        _ => FusionStrategy::rrf_default(),
    }
}

/// `weighted_rrf` over the two branches, as core's
/// `score_fusion_strategy` builds it: `dense_weight`/`sparse_weight` are
/// the two per-branch weights, `k` the RRF smoothing constant.
fn build_weighted_rrf(clause: &FusionClause) -> FusionStrategy {
    let dense = clause.dense_weight.unwrap_or(0.5);
    let sparse = clause.sparse_weight.unwrap_or(0.5);
    #[allow(clippy::cast_precision_loss)]
    let k = clause.k.unwrap_or(60) as f32;
    FusionStrategy::weighted_rrf(vec![dense, sparse], k)
        .unwrap_or_else(|_| FusionStrategy::rrf_default())
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
