//! Filter-strategy decision logic for `VelesQL` EXPLAIN.
//!
//! Extracted from `plan_builder.rs` to keep the file under the 500 NLOC limit
//! (Devin Finding G on PR #606). Public surface is `pub(super)` only — these
//! helpers are consumed exclusively by `plan_builder::QueryPlan`.
//!
//! # Pre-filter vs. post-filter trade-off
//!
//! For queries that combine a vector search with additional predicates, the
//! planner picks between two execution shapes:
//!
//! - **PreFilter**: scan every row and evaluate the predicate, then run HNSW
//!   on the surviving candidates. Cheap when the predicate is highly
//!   selective (e.g. `category = 'tech'` pruning 95 % of the rows).
//! - **PostFilter**: run HNSW on the full set, then evaluate the predicate on
//!   the top-k results. Cheap when the predicate is loose or cheap per-row.
//!
//! [`resolve_filter_strategy`] compares both costs using the calibrated
//! [`CostEstimator`] and picks the cheaper one, with a recall guardrail that
//! forces PostFilter when selectivity >= [`PREFILTER_RECALL_GUARD`].
//!
//! ## Why PreFilter is quasi-unreachable on large collections
//!
//! On collections with millions of rows, PreFilter is rarely chosen — this is
//! by design, not a bug. The pre-filter cost model is
//! `scan(total) + hnsw(total * sel)`, where `scan(total)` is linear in the
//! full row count. For collections where `total > 10 K`, the scan term
//! dominates almost any HNSW cost unless `selectivity < 1e-4`, which is
//! exceedingly rare in practice. The post-filter cost stays `hnsw(total) +
//! k * cpu_tuple_cost`, so PostFilter wins whenever the full-pass HNSW is
//! cheaper than scanning every row.
//!
//! Follow-up issue [#609](https://github.com/cyberlife-coder/velesdb/issues/609)
//! tracks a refinement: replace the rough `POSTFILTER_TOPK_COST_FRACTION`
//! approximation with a proper `k * cpu_tuple_cost` model that uses the real
//! `candidates` value and the calibrated `cpu_tuple_cost`. That refinement
//! will make the comparison tighter for intermediate collections
//! (10K – 1M rows) where the current model slightly overestimates post-filter
//! cost.

use std::sync::atomic::{AtomicU64, Ordering};

use super::types::FilterStrategy;
use crate::collection::stats::CollectionStats as CoreCollectionStats;
use crate::error::{Error, Result};
use crate::velesql::ast::{Condition, SelectStatement};
use crate::velesql::cost_estimator::{CostEstimator, SelectivityMethod};

/// Default selectivity threshold: `selectivity > 0.1 → PostFilter` when no
/// calibrated stats are available. Preserved bit-for-bit as a
/// backward-compatibility anchor for the ~50 pre-existing `EXPLAIN` tests
/// that predate calibrated costs (see `test_filter_strategy_post_filter_default`).
pub const DEFAULT_FALLBACK_SELECTIVITY_THRESHOLD: f64 = 0.1;

/// Runtime-tunable fallback threshold. Bit-encoded `f64` so reads on the
/// hot path (inside `resolve_filter_strategy`) are lock-free. Tune via
/// [`set_fallback_selectivity_threshold`]; read via
/// [`fallback_selectivity_threshold`].
static FALLBACK_SELECTIVITY_THRESHOLD_BITS: AtomicU64 =
    AtomicU64::new(DEFAULT_FALLBACK_SELECTIVITY_THRESHOLD.to_bits());

/// Returns the current fallback selectivity threshold (default `0.1`).
///
/// When no calibrated [`CollectionStats`](CoreCollectionStats) is available
/// (un-analyzed collection, SDK path without collection handle), the internal
/// `resolve_filter_strategy` helper switches from `PreFilter` to `PostFilter`
/// when the heuristic selectivity exceeds this threshold. The default value keeps
/// parity with the ~50 pre-existing EXPLAIN tests that predate calibrated
/// costs; tune for workloads where the calibrated pathway is unavailable.
#[must_use]
pub fn fallback_selectivity_threshold() -> f64 {
    f64::from_bits(FALLBACK_SELECTIVITY_THRESHOLD_BITS.load(Ordering::Relaxed))
}

/// Updates the fallback selectivity threshold at runtime. Returns the
/// previous value so the caller can restore it (useful in tests).
///
/// Accepts any finite value in `[0.0, 1.0]`. Reject path ensures the
/// threshold remains a valid probability so the caller cannot silently turn
/// the fallback into "always PreFilter" (threshold = +∞) or "always PostFilter"
/// (threshold < 0) — those regimes are better expressed via the calibrated
/// pathway.
///
/// # Example
/// ```
/// use velesdb_core::velesql::{
///     fallback_selectivity_threshold,
///     set_fallback_selectivity_threshold,
///     DEFAULT_FALLBACK_SELECTIVITY_THRESHOLD,
/// };
/// let previous = set_fallback_selectivity_threshold(0.3).expect("0.3 is in range");
/// assert_eq!(previous, DEFAULT_FALLBACK_SELECTIVITY_THRESHOLD);
/// assert!((fallback_selectivity_threshold() - 0.3).abs() < 1e-12);
/// // Restore for any downstream tests.
/// set_fallback_selectivity_threshold(DEFAULT_FALLBACK_SELECTIVITY_THRESHOLD).unwrap();
/// ```
///
/// # Errors
/// Returns [`crate::error::Error::Config`] when `value` is NaN, negative, or greater than
/// `1.0`.
pub fn set_fallback_selectivity_threshold(value: f64) -> Result<f64> {
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(Error::Config(format!(
            "fallback_selectivity_threshold must be a finite value in [0.0, 1.0], got {value}"
        )));
    }
    let previous = FALLBACK_SELECTIVITY_THRESHOLD_BITS.swap(value.to_bits(), Ordering::Relaxed);
    Ok(f64::from_bits(previous))
}

/// Guardrail: if a predicate filters out less than this fraction of rows,
/// running the pre-filter before HNSW is likely to hurt recall (too many
/// candidates survive). Forces PostFilter in that case.
pub(super) const PREFILTER_RECALL_GUARD: f64 = 0.5;

// POSTFILTER_TOPK_COST_FRACTION was removed in favour of
// `CostEstimator::estimate_post_filter_topk_cost(k)` (issue #609). The
// previous `filter_scan_cost × 0.01` approximation was off by up to 5× for
// large collections with selectivity near the recall guardrail. The
// calibrated formula `k × cpu_tuple_cost × cpu_ratio` models the physical
// reality of evaluating the predicate on the HNSW top-k tuples only,
// independent of collection size and selectivity.

/// Computes `(selectivity, estimation_method, estimated_rows)` for the
/// filter block. When `stats` is `Some` and the WHERE clause tree is
/// available, uses `CostEstimator::estimate_condition_selectivity_with_method`
/// on the non-vector subtree and reports the actual method that produced
/// the estimate (histogram / cardinality / heuristic — issue #471, Devin
/// finding 2). Otherwise falls back to the string-count heuristic supplied
/// by the caller via `heuristic_fallback`.
pub(super) fn estimate_filter_stats(
    stmt: &SelectStatement,
    heuristic_fallback: f64,
    stats: Option<&CoreCollectionStats>,
) -> (f64, Option<String>, Option<u64>) {
    match (stats, stmt.where_clause.as_ref()) {
        (Some(s), Some(where_cond)) => {
            let est = CostEstimator::new(s);
            let non_vector = strip_vector_predicates(where_cond);
            let (sel, method) = non_vector
                .as_ref()
                .map_or((1.0, SelectivityMethod::Heuristic), |c| {
                    est.estimate_condition_selectivity_with_method(c)
                });
            let total = s.total_points.max(s.row_count);
            // Reason: total*sel is a cardinality estimate; small fractional
            // losses after ceil are acceptable for EXPLAIN display.
            #[allow(
                clippy::cast_precision_loss,
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss
            )]
            let rows = ((total as f64) * sel).ceil() as u64;
            (sel, Some(method.as_str().to_string()), Some(rows))
        }
        _ => (heuristic_fallback, None, None),
    }
}

/// Returns the where-clause condition tree with `VectorSearch`-family
/// nodes removed, because selectivity of the non-vector predicates is
/// what drives the pre/post-filter decision.
///
/// Returns `None` when every branch is vector-related (selectivity = 1.0).
pub(super) fn strip_vector_predicates(condition: &Condition) -> Option<Condition> {
    match condition {
        Condition::VectorSearch(_)
        | Condition::VectorFusedSearch(_)
        | Condition::SparseVectorSearch(_)
        | Condition::Similarity(_) => None,
        Condition::And(left, right) => {
            match (
                strip_vector_predicates(left),
                strip_vector_predicates(right),
            ) {
                (Some(l), Some(r)) => Some(Condition::And(Box::new(l), Box::new(r))),
                (Some(l), None) => Some(l),
                (None, Some(r)) => Some(r),
                (None, None) => None,
            }
        }
        Condition::Or(left, right) => {
            match (
                strip_vector_predicates(left),
                strip_vector_predicates(right),
            ) {
                (Some(l), Some(r)) => Some(Condition::Or(Box::new(l), Box::new(r))),
                // If one branch is vector-only, the OR is satisfied by
                // the vector side too → treat as selectivity 1.0.
                _ => None,
            }
        }
        Condition::Not(inner) => {
            strip_vector_predicates(inner).map(|c| Condition::Not(Box::new(c)))
        }
        Condition::Group(inner) => {
            strip_vector_predicates(inner).map(|c| Condition::Group(Box::new(c)))
        }
        other => Some(other.clone()),
    }
}

/// Picks `PreFilter` vs `PostFilter` given a pre-computed selectivity,
/// the real `ef_search` / `candidates` from the query, and optional
/// calibrated stats.
///
/// When `stats` is `Some` and there's a vector search in the plan, runs
/// a cost comparison: pre-filter cost (scan + filter then HNSW on the
/// reduced set) vs post-filter cost (HNSW then filter on k results).
/// The HNSW cost uses `estimate_hnsw_search_cost_with_ef(ef_search,
/// candidates)` so the comparison reflects the user's actual WITH clause
/// (issue #471, Devin finding 4) instead of a hard-coded `k = 10`.
/// A recall guardrail forces `PostFilter` when selectivity is too loose
/// (>=0.5) so the pre-scan does not starve HNSW of good candidates.
///
/// When `stats` is `None`, preserves the historical 0.1 threshold.
pub(super) fn resolve_filter_strategy(
    selectivity: f64,
    has_vector_search: bool,
    ef_search: u32,
    candidates: u32,
    stats: Option<&CoreCollectionStats>,
) -> FilterStrategy {
    let Some(s) = stats else {
        return if selectivity > fallback_selectivity_threshold() {
            FilterStrategy::PostFilter
        } else {
            FilterStrategy::PreFilter
        };
    };

    if !has_vector_search {
        // No vector search → the notion of pre/post-filter doesn't apply
        // the same way; preserve the historical threshold for parity with
        // the non-vector plan shape.
        //
        // Observation (Devin finding on PR #606): for non-vector queries,
        // the threshold is applied to a calibrated histogram-based
        // selectivity once `stats` are present, but to the
        // `0.5^n`-style heuristic when `stats` is `None`. A highly
        // selective predicate such as `price < 10` (sel ≈ 0.01 after
        // ANALYZE vs. 0.5 without) therefore flips the reported
        // `filter_strategy` from PostFilter (heuristic) to PreFilter
        // (calibrated). This is purely informational for non-vector
        // plans — the execution engine does not dispatch on this
        // field when there is no HNSW stage — but is visible in
        // EXPLAIN output. Kept intentionally: the calibrated answer
        // is the semantically correct one.
        return if selectivity > fallback_selectivity_threshold() {
            FilterStrategy::PostFilter
        } else {
            FilterStrategy::PreFilter
        };
    }

    // Recall guardrail: a loose filter leaves too many candidates in the
    // HNSW frontier, so it's safer to post-filter after HNSW has done
    // the heavy lifting on the full set.
    if selectivity >= PREFILTER_RECALL_GUARD {
        return FilterStrategy::PostFilter;
    }

    let est = CostEstimator::new(s);
    let hnsw_cost = est
        .estimate_hnsw_search_cost_with_ef(ef_search, candidates)
        .total();
    // Pre-filter: evaluate the predicate on **every** row of the
    // collection (scan proportional to `total`, not `total*sel`) — then
    // run HNSW on the `sel*total` surviving candidates. HNSW on a
    // reduced set scales as `(ef + k) * log2(total*sel)`, not linearly
    // in the reduction factor, hence the dedicated
    // `estimate_hnsw_search_cost_with_ef_on_size` call (Devin finding E
    // on #606). The filter-scan component uses selectivity=1.0 so the
    // CBO does not under-estimate pre-filter cost by `1/selectivity`
    // (Devin finding A on #606).
    let total_points = s.total_points.max(s.row_count).max(1);
    // Reason: `total_points * selectivity` is a cardinality estimate;
    // floor to u64 is acceptable for a reduced-set size used in
    // `log2(size)` probe counting.
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    let reduced_size = ((total_points as f64) * selectivity).max(1.0) as u64;
    let hnsw_on_reduced = est
        .estimate_hnsw_search_cost_with_ef_on_size(ef_search, candidates, reduced_size)
        .total();
    let pre_filter = est.estimate_filter_cost_from_selectivity(1.0).total() + hnsw_on_reduced;
    // Post-filter: full HNSW pass, then filter evaluation on the HNSW
    // candidate set **before** top-k truncation. VelesDB's execution
    // (`search_post_filter` + `filter_and_hydrate`) runs the predicate on
    // the oversampled candidates and truncates to k afterwards, so the
    // cardinality of the filter evaluation is `max(k, ef_search)` — not
    // `k` alone. Modelling on k alone under-estimates the cost in the
    // typical `ef_search ≫ k` regime (Devin review on PR #612, issue
    // #609 closure).
    let post_filter = hnsw_cost
        + est
            .estimate_post_filter_topk_cost(candidates, ef_search)
            .total();

    if pre_filter < post_filter {
        FilterStrategy::PreFilter
    } else {
        FilterStrategy::PostFilter
    }
}
