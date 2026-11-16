//! Result fusion strategies for `VelesDB` WASM.
//!
//! Every strategy — `average`, `maximum`, `weighted`, `rrf`, `weighted_rrf`
//! and `relative_score` / `rsf` — is core's [`velesdb_core::FusionStrategy`],
//! so the browser engine and the core engine produce identical rankings.
//! The only production caller, `multi_query_search`, fuses one branch per
//! query vector; core's `relative_score` fuses every one of those N branches,
//! equal weights beyond two (D005: the browser used to average each
//! document's scores over the branches it appeared in, and core dropped
//! every branch past the second).
//!
//! The strategy *name* — its aliases, its case rule — is core's grammar,
//! [`velesdb_core::FusionStrategy::from_name`], whichever aggregation runs.
//!
//! `weighted` defaults: when no explicit weights are supplied, `fuse_results`
//! uses core's canonical `DEFAULT_WEIGHTED_*` constants
//! (`avg=0.6, max=0.3, hit=0.1`), as `from_name` fills them in. Callers may
//! override them via the `weights` parameter.

use velesdb_core::{FusionParams, FusionStrategy};

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
/// Returns core's error if `strategy` is not one of the names
/// [`FusionStrategy::from_name`] accepts (`"average"` / `"avg"`,
/// `"maximum"` / `"max"`, `"weighted"`, `"relative_score"` / `"rsf"`,
/// `"rrf"`, case-insensitive), or if `weights` are supplied for `"weighted"`
/// but are negative or do not sum to 1.0.
pub fn fuse_results(
    all_results: &[Vec<(u64, f32)>],
    strategy: &str,
    rrf_k: u32,
    weights: Option<(f32, f32, f32)>,
    branch_weights: Option<&[f32]>,
) -> Result<Vec<(u64, f32)>, String> {
    let (avg_weight, max_weight, hit_weight) = match weights {
        Some((avg, max, hit)) => (Some(avg), Some(max), Some(hit)),
        None => (None, None, None),
    };
    let params = FusionParams::default()
        .with_rrf_k(Some(rrf_k))
        .with_avg_weight(avg_weight)
        .with_max_weight(max_weight)
        .with_hit_weight(hit_weight);
    let core_strategy = FusionStrategy::from_name_with_weights(strategy, params, branch_weights)
        .map_err(|e| e.to_string())?;
    fuse_with_core(all_results, &core_strategy)
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

#[cfg(test)]
#[path = "fusion_tests.rs"]
mod tests;
