//! Result fusion strategies for `VelesDB` WASM.
//!
//! The four score/rank strategies (`average`, `maximum`, `weighted`, `rrf`)
//! delegate to the canonical [`velesdb_core::FusionStrategy`] so the browser
//! engine and the core engine produce identical rankings. The
//! `relative_score` / `rsf` strategy keeps a WASM-local *aggregation* (it
//! averages across N branches instead of core's fixed dense+sparse pair) but
//! its per-branch min-max normalization now delegates to
//! [`velesdb_core::fusion::min_max_normalize`] — see [`fuse_relative_score`].
//!
//! Branch-arity split (why `rsf` aggregation is *not* converged here): the
//! only production caller of [`fuse_results`] is `multi_query_search`, which
//! fuses one branch per query vector, so the branch count is the
//! user-supplied query count — genuinely N, with no dense/sparse distinction.
//! Core's `RelativeScore` is defined only for the 2-branch dense+sparse
//! hybrid (it discards branches beyond index 1), so delegating this N-branch
//! path to it would silently drop results. The 2-branch hybrid that *does*
//! match core's contract — the VelesQL `USING FUSION (strategy='rsf')` clause
//! — already routes through core (`crate::velesql_fusion::build_rsf` →
//! `FusionStrategy::relative_score`), so only the genuinely-N aggregation
//! stays WASM-local; the normalization math underneath it is single-sourced
//! from core (issue #1545).
//!
//! `weighted` defaults: when no explicit weights are supplied, `fuse_results`
//! uses `velesdb_core::FusionStrategy::weighted_default()`, i.e. core's
//! canonical `DEFAULT_WEIGHTED_*` constants (`avg=0.6, max=0.3, hit=0.1`).
//! Callers may override them via the `weights` parameter.

use std::collections::HashMap;

use velesdb_core::FusionStrategy;

/// Fuses results from multiple queries using the specified strategy.
///
/// # Arguments
///
/// * `all_results` - Results from each query as (id, score) pairs
/// * `strategy` - Fusion strategy: "average", "maximum", "weighted", or "rrf"
/// * `rrf_k` - RRF k parameter (typically 60)
/// * `weights` - Optional `(avg_weight, max_weight, hit_weight)` override for
///   the `"weighted"` strategy. When `None`, the canonical core defaults
///   ([`velesdb_core::FusionStrategy::weighted_default`]) are used. Ignored
///   for every other strategy.
///
/// # Returns
///
/// Fused results sorted by combined score (descending).
/// # Errors
///
/// Returns an error if `strategy` is not one of the recognised names:
/// `"average"` / `"avg"`, `"maximum"` / `"max"`, `"weighted"`,
/// `"relative_score"` / `"rsf"`, `"rrf"`; or if `weights` are supplied for
/// `"weighted"` but are negative or do not sum to 1.0.
pub fn fuse_results(
    all_results: &[Vec<(u64, f32)>],
    strategy: &str,
    rrf_k: u32,
    weights: Option<(f32, f32, f32)>,
) -> Result<Vec<(u64, f32)>, String> {
    match strategy.to_lowercase().as_str() {
        "average" | "avg" => fuse_with_core(all_results, &FusionStrategy::Average),
        "maximum" | "max" => fuse_with_core(all_results, &FusionStrategy::Maximum),
        "weighted" => {
            let weighted_strategy = match weights {
                Some((avg_weight, max_weight, hit_weight)) => {
                    FusionStrategy::weighted(avg_weight, max_weight, hit_weight)
                        .map_err(|e| e.to_string())?
                }
                None => FusionStrategy::weighted_default(),
            };
            fuse_with_core(all_results, &weighted_strategy)
        }
        "rrf" => fuse_with_core(all_results, &FusionStrategy::RRF { k: rrf_k }),
        "relative_score" | "rsf" => Ok(fuse_relative_score(all_results)),
        _ => Err(format!(
            "Unknown fusion strategy '{strategy}'. \
             Expected one of: average, avg, maximum, max, weighted, \
             relative_score, rsf, rrf"
        )),
    }
}

/// Delegates to the canonical core fusion and adapts its error to a `String`.
fn fuse_with_core(
    all_results: &[Vec<(u64, f32)>],
    strategy: &FusionStrategy,
) -> Result<Vec<(u64, f32)>, String> {
    strategy
        .fuse(all_results.to_vec())
        .map_err(|e| e.to_string())
}

/// Relative Score Fusion: min-max normalizes each query independently.
///
/// Each query's scores are normalized to `[0, 1]` via core's canonical
/// [`velesdb_core::fusion::min_max_normalize`] — the same single-sourced
/// helper `FusionStrategy::RelativeScore` uses internally — then averaged per
/// document across the queries in which the document appears. When all
/// scores in a branch are equal (range < epsilon), the normalized value
/// defaults to 0.5, per that helper's contract.
///
/// **Note:** the *aggregation* here is intentionally *not* delegated to
/// [`velesdb_core::FusionStrategy::RelativeScore`]. Core's `RelativeScore` is a
/// two-branch (dense + sparse) weighted sum that zero-fills documents missing
/// from a branch and discards branches beyond index 1. This WASM version
/// averages across N branches with equal weights and skips missing branches,
/// which yields a different ranking; converging the *aggregation* onto core
/// would silently change WASM search results. Only the per-branch
/// normalization math is shared (issue #1545).
fn fuse_relative_score(all_results: &[Vec<(u64, f32)>]) -> Vec<(u64, f32)> {
    let mut normalized: HashMap<u64, Vec<f32>> = HashMap::new();
    for results in all_results {
        for (id, norm) in velesdb_core::fusion::min_max_normalize(results) {
            normalized.entry(id).or_default().push(norm);
        }
    }

    let mut fused: Vec<(u64, f32)> = normalized
        .iter()
        .map(|(id, s)| {
            let avg = s.iter().sum::<f32>() / s.len() as f32;
            (*id, avg)
        })
        .collect();
    fused.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    fused
}

#[cfg(test)]
#[path = "fusion_tests.rs"]
mod tests;
