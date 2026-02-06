//! Cost model for query planning (EPIC-046 US-002).
//!
//! Provides cost estimation for different operation types based on
//! collection statistics, enabling cost-based query optimization.

// SAFETY: Numeric casts in cost model are intentional:
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

/// Cost factors for different operations (configurable).
///
/// These values are calibrated defaults that can be tuned based on
/// actual hardware characteristics.
#[derive(Debug, Clone)]
pub struct OperationCostFactors {
    /// Cost per sequential page access (8KB page)
    pub seq_page_cost: f64,
    /// Cost per random page access
    pub random_page_cost: f64,
    /// Cost per tuple/row processed
    pub cpu_tuple_cost: f64,
    /// Cost per index entry lookup
    pub cpu_index_cost: f64,
    /// Cost per vector distance calculation
    pub cpu_distance_cost: f64,
    /// Cost per graph edge traversal
    pub cpu_edge_cost: f64,
}

impl Default for OperationCostFactors {
    fn default() -> Self {
        Self {
            seq_page_cost: 1.0,
            random_page_cost: 4.0,
            cpu_tuple_cost: 0.01,
            cpu_index_cost: 0.005,
            cpu_distance_cost: 0.1,
            cpu_edge_cost: 0.02,
        }
    }
}

impl OperationCostFactors {
    /// Creates factors optimized for SSD storage.
    #[must_use]
    pub fn ssd_optimized() -> Self {
        Self {
            random_page_cost: 1.5, // SSDs have lower random access penalty
            ..Default::default()
        }
    }

    /// Creates factors optimized for in-memory operations.
    #[must_use]
    pub fn in_memory() -> Self {
        Self {
            seq_page_cost: 0.1,
            random_page_cost: 0.1,
            ..Default::default()
        }
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
        let io_cost = (index.depth as f64) * self.factors.random_page_cost;
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
