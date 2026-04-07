//! Cost model for query planning (EPIC-046 US-002).
//!
//! Provides cost estimation for different operation types based on
//! collection statistics, enabling cost-based query optimization.
//!
//! # Calibrated cost factors
//!
//! `OperationCostFactors` holds the per-operation weights used by the CBO.
//! Since Issue #467, these factors are **calibrated dynamically** from
//! [`CollectionStats`] during `analyze()` via [`calibrate_cost_factors()`],
//! replacing the former hard-coded constants. The calibration pipeline
//! adjusts each factor based on observed collection characteristics:
//!
//! - **`seq_page_cost`** — scaled by the page ratio (`total_size_bytes / 8 KB`)
//! - **`cpu_tuple_cost`** — scaled by average row size (`avg_row_size_bytes / 256`)
//! - **`cpu_index_cost`** — scaled by average column density from histograms
//! - **`random_page_cost`** — scaled by histogram skew (bucket imbalance)
//! - **stale penalty** — all factors inflated by 1.2× when any histogram is stale
//!
//! All calibrated values are clamped within [`CostFactorBounds`] to prevent
//! degenerate estimates. Hardware profiles (`ssd_optimized`, `in_memory`,
//! `hdd_optimized`) serve as the base before calibration adjustments.

// Reason: Numeric casts in cost model are intentional:
// - All casts are for cost estimation/statistics (not user data)
// - f64->f32 precision loss acceptable for query planning heuristics
// - f64->u64 sign loss acceptable (values are always positive costs/estimates)
// - u32->i32 for powi(): max_depth bounded by practical limits (< 1000)
// - Values are bounded by collection stats (cardinality, vector dimensions)
// - Cost estimates are approximate by design (order-of-magnitude accuracy)
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_possible_wrap)]

use crate::collection::stats::{CollectionStats, IndexStats};
use serde::{Deserialize, Serialize};
use tracing::debug;

/// Facteurs de coût pour les différentes opérations du CBO.
///
/// Ces valeurs sont calibrées dynamiquement à partir des statistiques de la
/// collection lors de `analyze()`. Les constructeurs statiques (`default()`,
/// `ssd_optimized()`, `in_memory()`, `hdd_optimized()`) fournissent des
/// bases pré-configurées pour différents profils matériels.
///
/// # Bornes de sécurité
///
/// Chaque facteur est borné dans un intervalle pour éviter les estimations
/// dégénérées (voir [`CostFactorBounds`]).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OperationCostFactors {
    /// Coût par accès séquentiel de page (8 KB). Borné dans \[0.01, 10.0\].
    #[serde(default = "default_seq_page_cost")]
    pub seq_page_cost: f64,
    /// Coût par accès aléatoire de page. Borné dans \[0.1, 20.0\].
    #[serde(default = "default_random_page_cost")]
    pub random_page_cost: f64,
    /// Coût CPU par tuple traité. Borné dans \[0.001, 0.1\].
    #[serde(default = "default_cpu_tuple_cost")]
    pub cpu_tuple_cost: f64,
    /// Coût CPU par lookup d'index. Borné dans \[0.001, 0.05\].
    #[serde(default = "default_cpu_index_cost")]
    pub cpu_index_cost: f64,
    /// Coût CPU par calcul de distance vectorielle. Borné dans \[0.01, 1.0\].
    #[serde(default = "default_cpu_distance_cost")]
    pub cpu_distance_cost: f64,
    /// Coût CPU par traversée d'arête de graphe. Borné dans \[0.005, 0.2\].
    #[serde(default = "default_cpu_edge_cost")]
    pub cpu_edge_cost: f64,
}

/// Returns the default value for `seq_page_cost`.
fn default_seq_page_cost() -> f64 {
    1.0
}

/// Returns the default value for `random_page_cost`.
fn default_random_page_cost() -> f64 {
    4.0
}

/// Returns the default value for `cpu_tuple_cost`.
fn default_cpu_tuple_cost() -> f64 {
    0.01
}

/// Returns the default value for `cpu_index_cost`.
fn default_cpu_index_cost() -> f64 {
    0.005
}

/// Returns the default value for `cpu_distance_cost`.
fn default_cpu_distance_cost() -> f64 {
    0.1
}

/// Returns the default value for `cpu_edge_cost`.
fn default_cpu_edge_cost() -> f64 {
    0.02
}

impl Default for OperationCostFactors {
    fn default() -> Self {
        Self {
            seq_page_cost: default_seq_page_cost(),
            random_page_cost: default_random_page_cost(),
            cpu_tuple_cost: default_cpu_tuple_cost(),
            cpu_index_cost: default_cpu_index_cost(),
            cpu_distance_cost: default_cpu_distance_cost(),
            cpu_edge_cost: default_cpu_edge_cost(),
        }
    }
}

/// Bornes de sécurité pour les facteurs de coût calibrés.
///
/// Empêche les estimations dégénérées causées par des statistiques aberrantes.
/// Chaque borne est un tuple `(min, max)` inclusif.
pub(crate) struct CostFactorBounds;

impl CostFactorBounds {
    /// Bornes pour `seq_page_cost`.
    pub const SEQ_PAGE_COST: (f64, f64) = (0.01, 10.0);
    /// Bornes pour `random_page_cost`.
    pub const RANDOM_PAGE_COST: (f64, f64) = (0.1, 20.0);
    /// Bornes pour `cpu_tuple_cost`.
    pub const CPU_TUPLE_COST: (f64, f64) = (0.001, 0.1);
    /// Bornes pour `cpu_index_cost`.
    pub const CPU_INDEX_COST: (f64, f64) = (0.001, 0.05);
    /// Bornes pour `cpu_distance_cost`.
    pub const CPU_DISTANCE_COST: (f64, f64) = (0.01, 1.0);
    /// Bornes pour `cpu_edge_cost`.
    pub const CPU_EDGE_COST: (f64, f64) = (0.005, 0.2);
}

/// Clamps `value` into `[min, max]` and emits a `debug!` log if clamped.
fn clamp_with_log(name: &str, value: f64, bounds: (f64, f64)) -> f64 {
    let clamped = value.clamp(bounds.0, bounds.1);
    if (clamped - value).abs() > f64::EPSILON {
        debug!(
            field = name,
            original = value,
            clamped = clamped,
            "cost factor clamped to bounds"
        );
    }
    clamped
}

impl OperationCostFactors {
    /// Creates factors optimized for SSD storage.
    ///
    /// SSDs have lower random access penalty compared to HDDs.
    #[must_use]
    pub fn ssd_optimized() -> Self {
        Self {
            random_page_cost: 1.5,
            ..Default::default()
        }
    }

    /// Creates factors optimized for in-memory operations.
    ///
    /// Both sequential and random page costs are minimal.
    #[must_use]
    pub fn in_memory() -> Self {
        Self {
            seq_page_cost: 0.1,
            random_page_cost: 0.1,
            ..Default::default()
        }
    }

    /// Creates factors optimized for HDD storage (rotational disks).
    ///
    /// `random_page_cost = 8.0` reflects the seek latency of rotational disks.
    /// `seq_page_cost = 1.0` remains standard (sequential reads are efficient on HDD).
    #[must_use]
    pub fn hdd_optimized() -> Self {
        Self {
            random_page_cost: 8.0,
            ..Default::default()
        }
    }

    /// Applies safety bounds to all cost factors.
    ///
    /// Each factor is clamped into its allowed interval defined by
    /// [`CostFactorBounds`]. Emits a `debug!` log for every clamped field.
    #[must_use]
    pub fn clamped(self) -> Self {
        Self {
            seq_page_cost: clamp_with_log(
                "seq_page_cost",
                self.seq_page_cost,
                CostFactorBounds::SEQ_PAGE_COST,
            ),
            random_page_cost: clamp_with_log(
                "random_page_cost",
                self.random_page_cost,
                CostFactorBounds::RANDOM_PAGE_COST,
            ),
            cpu_tuple_cost: clamp_with_log(
                "cpu_tuple_cost",
                self.cpu_tuple_cost,
                CostFactorBounds::CPU_TUPLE_COST,
            ),
            cpu_index_cost: clamp_with_log(
                "cpu_index_cost",
                self.cpu_index_cost,
                CostFactorBounds::CPU_INDEX_COST,
            ),
            cpu_distance_cost: clamp_with_log(
                "cpu_distance_cost",
                self.cpu_distance_cost,
                CostFactorBounds::CPU_DISTANCE_COST,
            ),
            cpu_edge_cost: clamp_with_log(
                "cpu_edge_cost",
                self.cpu_edge_cost,
                CostFactorBounds::CPU_EDGE_COST,
            ),
        }
    }

    /// Returns `true` if all factors equal the default values.
    #[must_use]
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// Estimated cost of an operation.
#[derive(Debug, Clone, Copy, Default)]
pub struct OperationCost {
    /// Startup cost (one-time initialization)
    pub startup: f64,
    /// Total cost including startup
    pub total: f64,
    /// Estimated rows returned
    pub rows: u64,
}

impl OperationCost {
    /// Creates a new cost estimate.
    #[must_use]
    pub fn new(startup: f64, total: f64, rows: u64) -> Self {
        Self {
            startup,
            total,
            rows,
        }
    }

    /// Combines two costs (sequential operations).
    #[must_use]
    pub fn then(self, next: OperationCost) -> Self {
        Self {
            startup: self.startup,
            total: self.total + next.total,
            rows: next.rows,
        }
    }
}

impl std::fmt::Display for OperationCost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Cost {{ startup: {:.2}, total: {:.2}, rows: {} }}",
            self.startup, self.total, self.rows
        )
    }
}

/// Cost estimator using collection statistics.
#[derive(Debug, Clone)]
pub struct CostEstimator {
    factors: OperationCostFactors,
    page_size: u64,
}

impl Default for CostEstimator {
    fn default() -> Self {
        Self::new(OperationCostFactors::default())
    }
}

impl CostEstimator {
    /// Creates a new cost estimator with given factors.
    #[must_use]
    pub fn new(factors: OperationCostFactors) -> Self {
        Self {
            factors,
            page_size: 8192, // 8KB default page size
        }
    }

    /// Estimates cost of a full sequential scan.
    #[must_use]
    pub fn estimate_scan(&self, stats: &CollectionStats) -> OperationCost {
        let pages = (stats.total_size_bytes as f64 / self.page_size as f64).ceil();
        let io_cost = pages * self.factors.seq_page_cost;
        let cpu_cost = stats.row_count as f64 * self.factors.cpu_tuple_cost;

        OperationCost {
            startup: 0.0,
            total: io_cost + cpu_cost,
            rows: stats.live_row_count(),
        }
    }

    /// Estimates cost of an index lookup with given selectivity.
    #[must_use]
    pub fn estimate_index_lookup(&self, index: &IndexStats, selectivity: f64) -> OperationCost {
        let selectivity = selectivity.clamp(0.0001, 1.0);
        let entries = (index.entry_count as f64 * selectivity) as u64;
        let io_cost = f64::from(index.depth) * self.factors.random_page_cost;
        let cpu_cost = entries as f64 * self.factors.cpu_index_cost;

        OperationCost {
            startup: io_cost,
            total: io_cost + cpu_cost,
            rows: entries.max(1),
        }
    }

    /// Estimates cost of HNSW vector search.
    #[must_use]
    pub fn estimate_vector_search(
        &self,
        k: u64,
        ef_search: u64,
        dataset_size: u64,
    ) -> OperationCost {
        // HNSW complexity: O(ef_search * log(n))
        let log_n = if dataset_size > 1 {
            (dataset_size as f64).log2()
        } else {
            1.0
        };
        let distances = (ef_search as f64 * log_n) as u64;
        let cpu_cost = distances as f64 * self.factors.cpu_distance_cost;

        OperationCost {
            startup: cpu_cost * 0.1,
            total: cpu_cost,
            rows: k,
        }
    }

    /// Estimates cost of graph traversal (BFS/DFS).
    #[must_use]
    pub fn estimate_graph_traversal(
        &self,
        avg_degree: f64,
        max_depth: u32,
        limit: u64,
    ) -> OperationCost {
        // Worst case: branching factor ^ depth, capped by limit
        let max_nodes = (avg_degree.powi(max_depth as i32) as u64).min(limit.saturating_mul(10));
        let edges = max_nodes as f64 * avg_degree;
        let cpu_cost = edges * self.factors.cpu_edge_cost;

        OperationCost {
            startup: 0.0,
            total: cpu_cost,
            rows: limit,
        }
    }

    /// Estimates cost of filter application.
    #[must_use]
    pub fn estimate_filter(&self, input_rows: u64, selectivity: f64) -> OperationCost {
        let selectivity = selectivity.clamp(0.0001, 1.0);
        let cpu_cost = input_rows as f64 * self.factors.cpu_tuple_cost;
        let output_rows = (input_rows as f64 * selectivity) as u64;

        OperationCost {
            startup: 0.0,
            total: cpu_cost,
            rows: output_rows.max(1),
        }
    }

    /// Compares two costs and returns the cheaper one.
    #[must_use]
    pub fn cheaper<'a>(&self, a: &'a OperationCost, b: &'a OperationCost) -> &'a OperationCost {
        if a.total <= b.total {
            a
        } else {
            b
        }
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scan_cost_scales_with_size() {
        let estimator = CostEstimator::default();

        let small = CollectionStats::with_counts(1_000, 0);
        let large = CollectionStats::with_counts(100_000, 0);

        let small_cost = estimator.estimate_scan(&small);
        let large_cost = estimator.estimate_scan(&large);

        assert!(large_cost.total > small_cost.total);
        assert_eq!(small_cost.rows, 1_000);
        assert_eq!(large_cost.rows, 100_000);
    }

    #[test]
    fn test_index_lookup_cheaper_than_scan() {
        let estimator = CostEstimator::default();

        let mut stats = CollectionStats::with_counts(100_000, 0);
        stats.total_size_bytes = 100_000 * 256; // 256 bytes per row

        let index = IndexStats::new("pk", "BTree")
            .with_entry_count(100_000)
            .with_depth(4);

        let scan_cost = estimator.estimate_scan(&stats);
        let index_cost = estimator.estimate_index_lookup(&index, 0.01); // 1% selectivity

        assert!(
            index_cost.total < scan_cost.total,
            "Index lookup should be cheaper than scan"
        );
    }

    #[test]
    fn test_vector_search_cost() {
        let estimator = CostEstimator::default();

        let cost = estimator.estimate_vector_search(10, 100, 100_000);

        assert!(cost.total > 0.0);
        assert_eq!(cost.rows, 10);
        assert!(cost.startup < cost.total);
    }

    #[test]
    fn test_graph_traversal_cost() {
        let estimator = CostEstimator::default();

        let cost = estimator.estimate_graph_traversal(5.0, 3, 100);

        assert!(cost.total > 0.0);
        assert_eq!(cost.rows, 100);
    }

    #[test]
    fn test_filter_reduces_rows() {
        let estimator = CostEstimator::default();

        let cost = estimator.estimate_filter(10_000, 0.1);

        assert_eq!(cost.rows, 1_000);
    }

    #[test]
    fn test_cost_comparison() {
        let estimator = CostEstimator::default();

        let cheap = OperationCost::new(0.0, 10.0, 100);
        let expensive = OperationCost::new(0.0, 100.0, 100);

        let winner = estimator.cheaper(&cheap, &expensive);
        assert!((winner.total - 10.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_cost_chaining() {
        let scan = OperationCost::new(0.0, 100.0, 10_000);
        let filter = OperationCost::new(0.0, 10.0, 1_000);

        let combined = scan.then(filter);

        assert!((combined.total - 110.0).abs() < f64::EPSILON);
        assert_eq!(combined.rows, 1_000);
    }

    #[test]
    fn test_ssd_optimized_factors() {
        let factors = OperationCostFactors::ssd_optimized();
        assert!(factors.random_page_cost < OperationCostFactors::default().random_page_cost);
    }
}
