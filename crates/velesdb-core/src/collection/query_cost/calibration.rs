//! Calibration pipeline for query cost factors (EPIC-046 US-002, Task 3.x).
//!
//! Derives I/O and CPU weights from observed collection characteristics
//! (size, row width, column density, histogram skew, staleness) instead of
//! relying on hard-coded constants. Extracted from `cost_model.rs` to keep
//! both files under the 500-NLOC limit.

// Reason: Numeric casts in calibration are intentional:
// - All casts are for cost estimation/statistics (not user data)
// - f64 precision loss acceptable for query planning heuristics
// - Values are bounded by collection stats (cardinality, vector dimensions)
// - Cost estimates are approximate by design (order-of-magnitude accuracy)
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_sign_loss)]

use crate::collection::stats::CollectionStats;
use tracing::debug;

use super::cost_factors::OperationCostFactors;

// ---------------------------------------------------------------------------
// Calibration constants
// ---------------------------------------------------------------------------

/// Default page size in bytes (8 KB).
const PAGE_SIZE: f64 = 8192.0;

/// Skew threshold above which `random_page_cost` is increased.
const SKEW_THRESHOLD: f64 = 10.0;

/// Penalty multiplier applied to all factors when any histogram is stale.
const STALE_PENALTY: f64 = 1.2;

// ---------------------------------------------------------------------------
// Calibration sub-functions (Task 3.1)
// ---------------------------------------------------------------------------

/// Adjusts `seq_page_cost` based on the page ratio of the collection.
///
/// Computes `ratio_pages = total_size_bytes / PAGE_SIZE`. For collections
/// that fit in memory (`ratio_pages < 1.0`), the cost is reduced
/// proportionally. The adjustment factor is clamped to `[0.1, 1.0]`.
///
/// # Formula
///
/// ```text
/// ratio_pages = total_size_bytes / 8192
/// factor = clamp(ratio_pages / max(ratio_pages, 1.0), 0.1, 1.0)
/// result = base * factor
/// ```
pub(crate) fn adjust_seq_page_cost(base: f64, stats: &CollectionStats) -> f64 {
    let ratio_pages = stats.total_size_bytes as f64 / PAGE_SIZE;
    let divisor = ratio_pages.max(1.0);
    let factor = (ratio_pages / divisor).clamp(0.1, 1.0);
    debug!(
        ratio_pages = ratio_pages,
        factor = factor,
        "adjust_seq_page_cost"
    );
    base * factor
}

/// Adjusts `cpu_tuple_cost` based on average row size.
///
/// Larger rows require more CPU to process. The row-size factor is
/// `avg_row_size_bytes / 256.0`, clamped to `[0.5, 4.0]`.
/// When `avg_row_size_bytes` is 0, returns `base` unchanged (Req 9.4).
///
/// # Formula
///
/// ```text
/// row_size_factor = clamp(avg_row_size_bytes / 256.0, 0.5, 4.0)
/// result = base * row_size_factor
/// ```
pub(crate) fn adjust_cpu_tuple_cost(base: f64, stats: &CollectionStats) -> f64 {
    if stats.avg_row_size_bytes == 0 {
        debug!("avg_row_size_bytes is 0, skipping cpu_tuple_cost adjustment");
        return base;
    }
    let row_size_factor = (stats.avg_row_size_bytes as f64 / 256.0).clamp(0.5, 4.0);
    debug!(
        avg_row_size = stats.avg_row_size_bytes,
        factor = row_size_factor,
        "adjust_cpu_tuple_cost"
    );
    base * row_size_factor
}

/// Adjusts `cpu_index_cost` based on average column density from histograms.
///
/// Density is `avg(distinct_count / total_rows)` across columns that have a
/// histogram. Low-density columns (many duplicates) make indexes less
/// selective, increasing the cost. When no histograms are available, returns
/// `base` unchanged.
///
/// # Formula
///
/// ```text
/// density = avg(distinct_count / total_rows) for columns with histogram
/// factor = clamp(1.0 + (1.0 - density), 1.0, 3.0)
/// result = base * factor
/// ```
pub(crate) fn adjust_cpu_index_cost(base: f64, stats: &CollectionStats) -> f64 {
    let density = compute_avg_density(stats);
    if let Some(d) = density {
        let factor = (1.0 + (1.0 - d)).clamp(1.0, 3.0);
        debug!(density = d, factor = factor, "adjust_cpu_index_cost");
        base * factor
    } else {
        debug!("no histograms available, skipping cpu_index_cost adjustment");
        base
    }
}

/// Computes the average column density across all columns with a histogram.
///
/// Returns `None` when no column has a non-empty histogram with
/// `total_count > 0`.
fn compute_avg_density(stats: &CollectionStats) -> Option<f64> {
    let mut sum = 0.0_f64;
    let mut count = 0_u64;
    for col in stats.column_stats.values() {
        if let Some(hist) = col.histogram.as_ref() {
            if hist.total_count > 0 && !hist.buckets.is_empty() {
                let distinct: u64 = hist.buckets.iter().map(|b| b.distinct_count).sum();
                let density = distinct as f64 / hist.total_count as f64;
                sum += density.clamp(0.0, 1.0);
                count += 1;
            }
        }
    }
    if count == 0 {
        None
    } else {
        Some(sum / count as f64)
    }
}

/// Adjusts `random_page_cost` based on histogram skew.
///
/// Skew is `max(max_bucket_count / min_bucket_count)` across all histograms.
/// When skew exceeds `SKEW_THRESHOLD` (10.0), the cost is increased using
/// `log2(skew / threshold)`. Buckets with `count == 0` are skipped to avoid
/// division by zero.
///
/// # Formula
///
/// ```text
/// skew = max(max_bucket / min_bucket) across all histograms
/// if skew > 10.0:
///   factor = clamp(1.0 + log2(skew / 10.0), 1.0, 2.0)
///   result = base * factor
/// else:
///   result = base
/// ```
pub(crate) fn adjust_for_skew(base: f64, stats: &CollectionStats) -> f64 {
    let skew = compute_max_skew(stats);
    if let Some(s) = skew {
        if s > SKEW_THRESHOLD {
            let factor = (1.0 + (s / SKEW_THRESHOLD).log2()).clamp(1.0, 2.0);
            debug!(skew = s, factor = factor, "adjust_for_skew");
            return base * factor;
        }
    }
    debug!("skew below threshold or no histograms, no adjustment");
    base
}

/// Computes the maximum skew ratio across all histograms.
///
/// Returns `None` when no histogram has at least two buckets with non-zero
/// counts.
fn compute_max_skew(stats: &CollectionStats) -> Option<f64> {
    let mut max_skew: Option<f64> = None;
    for col in stats.column_stats.values() {
        if let Some(hist) = col.histogram.as_ref() {
            let non_zero: Vec<u64> = hist
                .buckets
                .iter()
                .map(|b| b.count)
                .filter(|&c| c > 0)
                .collect();
            if non_zero.len() >= 2 {
                let max_b = non_zero.iter().copied().max().unwrap_or(1);
                let min_b = non_zero.iter().copied().min().unwrap_or(1);
                if min_b > 0 {
                    let skew = max_b as f64 / min_b as f64;
                    max_skew = Some(max_skew.map_or(skew, |prev: f64| prev.max(skew)));
                }
            }
        }
    }
    max_skew
}

/// Multiplies all cost factors by the given penalty.
///
/// Used to inflate costs when histogram data is stale, reflecting the
/// increased uncertainty in cost estimates.
pub(crate) fn apply_stale_penalty(
    factors: &OperationCostFactors,
    penalty: f64,
) -> OperationCostFactors {
    debug!(penalty = penalty, "applying stale histogram penalty");
    OperationCostFactors {
        seq_page_cost: factors.seq_page_cost * penalty,
        random_page_cost: factors.random_page_cost * penalty,
        cpu_tuple_cost: factors.cpu_tuple_cost * penalty,
        cpu_index_cost: factors.cpu_index_cost * penalty,
        cpu_distance_cost: factors.cpu_distance_cost * penalty,
        cpu_edge_cost: factors.cpu_edge_cost * penalty,
    }
}

/// Returns `true` if any histogram in the collection stats is marked stale.
pub(crate) fn has_stale_histogram(stats: &CollectionStats) -> bool {
    stats
        .column_stats
        .values()
        .any(|col| col.histogram.as_ref().is_some_and(|h| h.stale))
}

// ---------------------------------------------------------------------------
// Calibration orchestrator (Task 3.2)
// ---------------------------------------------------------------------------

/// Calibrates cost factors from collection statistics.
///
/// Derives I/O and CPU weights from observed collection characteristics
/// (size, row width, column density, histogram skew, staleness) instead of
/// relying on hard-coded constants.
///
/// # Algorithm
///
/// 1. If `row_count == 0` → return `OperationCostFactors::default()`
/// 2. Adjust `seq_page_cost` from page ratio
/// 3. Adjust `cpu_tuple_cost` from average row size
/// 4. Adjust `cpu_index_cost` from average column density
/// 5. Adjust `random_page_cost` from histogram skew
/// 6. If any histogram is stale → multiply all factors by `STALE_PENALTY`
/// 7. Clamp all factors within `CostFactorBounds`
pub(crate) fn calibrate_cost_factors(
    stats: &CollectionStats,
    base: &OperationCostFactors,
) -> OperationCostFactors {
    if stats.row_count == 0 {
        debug!("row_count is 0, returning default cost factors");
        return OperationCostFactors::default();
    }

    let mut factors = base.clone();
    factors.seq_page_cost = adjust_seq_page_cost(base.seq_page_cost, stats);
    factors.cpu_tuple_cost = adjust_cpu_tuple_cost(base.cpu_tuple_cost, stats);
    factors.cpu_index_cost = adjust_cpu_index_cost(base.cpu_index_cost, stats);
    factors.random_page_cost = adjust_for_skew(base.random_page_cost, stats);

    if has_stale_histogram(stats) {
        factors = apply_stale_penalty(&factors, STALE_PENALTY);
    }

    factors.clamped()
}
