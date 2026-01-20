//! Query Planner for hybrid MATCH + NEAR queries.
//!
//! This module provides intelligent query planning for hybrid graph-vector queries,
//! choosing the optimal execution strategy based on estimated selectivity.

use std::sync::atomic::{AtomicU64, Ordering};

/// Execution strategy for hybrid queries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionStrategy {
    /// Execute vector search first, then filter by graph pattern.
    /// Best when graph filter is not very selective (>10% of data).
    VectorFirst,
    /// Execute graph pattern first, then vector search on candidates.
    /// Best when graph filter is very selective (<1% of data).
    GraphFirst,
    /// Execute both in parallel and merge results.
    /// Best for medium selectivity (1-10% of data).
    Parallel,
}

/// Statistics for query planning decisions.
#[derive(Debug, Default)]
pub struct QueryStats {
    /// Estimated ratio of nodes matching graph patterns (0.0-1.0).
    graph_selectivity: AtomicU64,
    /// Average vector search latency in microseconds.
    avg_vector_latency_us: AtomicU64,
    /// Average graph traversal latency in microseconds.
    avg_graph_latency_us: AtomicU64,
    /// Number of vector queries executed (for averaging).
    vector_query_count: AtomicU64,
    /// Number of graph queries executed (for averaging).
    graph_query_count: AtomicU64,
}

impl QueryStats {
    /// Creates new empty query statistics.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Updates graph selectivity estimate.
    pub fn update_graph_selectivity(&self, matched: u64, total: u64) {
        if total > 0 {
            let selectivity = (matched as f64 / total as f64 * 1_000_000.0) as u64;
            self.graph_selectivity.store(selectivity, Ordering::Relaxed);
        }
    }

    /// Gets the current graph selectivity estimate (0.0-1.0).
    #[must_use]
    pub fn graph_selectivity(&self) -> f64 {
        self.graph_selectivity.load(Ordering::Relaxed) as f64 / 1_000_000.0
    }

    /// Updates average vector search latency using exponential moving average.
    ///
    /// Uses EMA with α=0.1 for thread-safe updates without race conditions.
    /// EMA formula: new_avg = α * latency + (1-α) * old_avg
    /// This avoids the race condition in running average calculations.
    pub fn update_vector_latency(&self, latency_us: u64) {
        self.vector_query_count.fetch_add(1, Ordering::Relaxed);
        Self::atomic_ema_update(&self.avg_vector_latency_us, latency_us);
    }

    /// Updates average graph traversal latency using exponential moving average.
    ///
    /// Uses EMA with α=0.1 for thread-safe updates without race conditions.
    /// This ensures accurate statistics for query planning decisions.
    pub fn update_graph_latency(&self, latency_us: u64) {
        self.graph_query_count.fetch_add(1, Ordering::Relaxed);
        Self::atomic_ema_update(&self.avg_graph_latency_us, latency_us);
    }

    /// Atomically updates an EMA using compare-and-swap loop.
    ///
    /// α = 0.1 (10% weight to new value, 90% to historical average)
    /// This provides smooth averaging while being fully thread-safe.
    fn atomic_ema_update(avg: &AtomicU64, new_value: u64) {
        loop {
            let old_avg = avg.load(Ordering::Relaxed);
            let new_avg = if old_avg == 0 {
                // First value: use it directly
                new_value
            } else {
                // EMA: new_avg = 0.1 * new_value + 0.9 * old_avg
                // Using integer math: (new_value + 9 * old_avg) / 10
                (new_value + 9 * old_avg) / 10
            };
            // CAS loop ensures atomic read-modify-write
            if avg
                .compare_exchange_weak(old_avg, new_avg, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                break;
            }
            // Retry on contention
        }
    }

    /// Gets the average vector latency in microseconds.
    #[must_use]
    pub fn avg_vector_latency_us(&self) -> u64 {
        self.avg_vector_latency_us.load(Ordering::Relaxed)
    }

    /// Gets the average graph latency in microseconds.
    #[must_use]
    pub fn avg_graph_latency_us(&self) -> u64 {
        self.avg_graph_latency_us.load(Ordering::Relaxed)
    }

    /// Gets the total number of vector queries.
    #[must_use]
    pub fn vector_query_count(&self) -> u64 {
        self.vector_query_count.load(Ordering::Relaxed)
    }

    /// Gets the total number of graph queries.
    #[must_use]
    pub fn graph_query_count(&self) -> u64 {
        self.graph_query_count.load(Ordering::Relaxed)
    }
}

/// Query planner for hybrid MATCH + NEAR queries.
#[derive(Debug, Default)]
pub struct QueryPlanner {
    /// Runtime statistics for adaptive planning.
    stats: QueryStats,
    /// Selectivity threshold for GraphFirst strategy.
    graph_first_threshold: f64,
    /// Selectivity threshold for VectorFirst strategy.
    vector_first_threshold: f64,
}

impl QueryPlanner {
    /// Creates a new query planner with default thresholds.
    #[must_use]
    pub fn new() -> Self {
        Self {
            stats: QueryStats::new(),
            graph_first_threshold: 0.01,  // <1% → GraphFirst
            vector_first_threshold: 0.50, // >50% → VectorFirst
        }
    }

    /// Creates a planner with custom selectivity thresholds.
    #[must_use]
    pub fn with_thresholds(graph_first: f64, vector_first: f64) -> Self {
        Self {
            stats: QueryStats::new(),
            graph_first_threshold: graph_first,
            vector_first_threshold: vector_first,
        }
    }

    /// Chooses the optimal execution strategy based on estimated selectivity.
    #[must_use]
    pub fn choose_strategy(&self, estimated_selectivity: Option<f64>) -> ExecutionStrategy {
        let selectivity = estimated_selectivity.unwrap_or_else(|| self.stats.graph_selectivity());

        if selectivity < self.graph_first_threshold {
            ExecutionStrategy::GraphFirst
        } else if selectivity > self.vector_first_threshold {
            ExecutionStrategy::VectorFirst
        } else {
            ExecutionStrategy::Parallel
        }
    }

    /// Returns a reference to the query statistics.
    #[must_use]
    pub fn stats(&self) -> &QueryStats {
        &self.stats
    }

    /// Estimates selectivity based on label and relationship type counts.
    ///
    /// This is a heuristic based on the principle that:
    /// - Rare labels/types → low selectivity → GraphFirst
    /// - Common labels/types → high selectivity → VectorFirst
    #[must_use]
    pub fn estimate_selectivity(
        &self,
        label_count: u64,
        total_nodes: u64,
        rel_type_count: u64,
        total_edges: u64,
    ) -> f64 {
        if total_nodes == 0 {
            return 1.0; // No data → assume all match
        }

        let label_sel = if total_nodes > 0 {
            label_count as f64 / total_nodes as f64
        } else {
            1.0
        };

        let rel_sel = if total_edges > 0 && rel_type_count > 0 {
            rel_type_count as f64 / total_edges as f64
        } else {
            1.0
        };

        // Combined selectivity (multiplicative for AND)
        label_sel * rel_sel
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_choose_strategy_graph_first() {
        let planner = QueryPlanner::new();
        assert_eq!(
            planner.choose_strategy(Some(0.005)),
            ExecutionStrategy::GraphFirst
        );
    }

    #[test]
    fn test_choose_strategy_vector_first() {
        let planner = QueryPlanner::new();
        assert_eq!(
            planner.choose_strategy(Some(0.8)),
            ExecutionStrategy::VectorFirst
        );
    }

    #[test]
    fn test_choose_strategy_parallel() {
        let planner = QueryPlanner::new();
        assert_eq!(
            planner.choose_strategy(Some(0.2)),
            ExecutionStrategy::Parallel
        );
    }

    #[test]
    fn test_estimate_selectivity() {
        let planner = QueryPlanner::new();

        // 10 persons out of 1000 nodes, 5 WROTE edges out of 100 edges
        let sel = planner.estimate_selectivity(10, 1000, 5, 100);
        assert!((sel - 0.0005).abs() < 0.0001);
    }

    #[test]
    fn test_query_stats_update() {
        let stats = QueryStats::new();

        stats.update_graph_selectivity(10, 1000);
        assert!((stats.graph_selectivity() - 0.01).abs() < 0.0001);

        stats.update_vector_latency(100);
        stats.update_vector_latency(200);
        // EMA with α=0.1: first=100, then (200 + 9*100)/10 = 110
        assert_eq!(stats.avg_vector_latency_us(), 110);
        assert_eq!(stats.vector_query_count(), 2);
    }

    #[test]
    fn test_graph_latency_independent_count() {
        let stats = QueryStats::new();

        // Update vector latency multiple times
        stats.update_vector_latency(100);
        stats.update_vector_latency(200);
        stats.update_vector_latency(300);

        // Graph latency should use its own counter, not vector_query_count
        stats.update_graph_latency(50);
        stats.update_graph_latency(150);

        // EMA with α=0.1: first=50, then (150 + 9*50)/10 = 60
        // Graph stats are independent from vector stats
        assert_eq!(stats.avg_graph_latency_us(), 60);
        assert_eq!(stats.graph_query_count(), 2);
        assert_eq!(stats.vector_query_count(), 3);
    }

    #[test]
    fn test_custom_thresholds() {
        let planner = QueryPlanner::with_thresholds(0.05, 0.30);

        assert_eq!(
            planner.choose_strategy(Some(0.03)),
            ExecutionStrategy::GraphFirst
        );
        assert_eq!(
            planner.choose_strategy(Some(0.15)),
            ExecutionStrategy::Parallel
        );
        assert_eq!(
            planner.choose_strategy(Some(0.40)),
            ExecutionStrategy::VectorFirst
        );
    }
}
