//! Query Planner for hybrid MATCH + NEAR queries.
//!
//! This module provides intelligent query planning for hybrid graph-vector queries,
//! choosing the optimal execution strategy based on estimated selectivity.
//!
//! # Future: Cost-Based Optimization (v2.0)
//!
//! The current planner uses heuristic-based strategy selection. Future improvements:
//! - Collect runtime statistics for actual selectivity estimation
//! - Implement cost model based on index cardinality
//! - Add adaptive query execution with plan switching

// Reason: Numeric casts across this file are intentional and bounded:
// - u64/usize → f64: cardinalities used in planning heuristics; ±1 ULP is operationally irrelevant.
// - f64 → usize: over-fetch factor clamped to [1.0, 64.0] before cast; no truncation possible.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

use crate::collection::stats::CollectionStats;
use crate::velesql::ast::Condition;
pub use crate::velesql::cost_estimator::{Cost, CostEstimator};
pub use crate::velesql::query_stats::QueryStats;

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

const VECTOR_FIRST_FILTER_PENALTY: f64 = 1.5;
const PARALLEL_MERGE_OVERHEAD: f64 = 25.0;
const GRAPH_TO_VECTOR_SCALING: f64 = 100.0;

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

    /// Chooses strategy using CBO with collection statistics and optional filter.
    #[must_use]
    pub fn choose_strategy_with_cbo(
        &self,
        stats: &CollectionStats,
        filter: Option<&Condition>,
        k: usize,
    ) -> ExecutionStrategy {
        // Without metadata/graph filter, vector-first is the only meaningful strategy.
        if filter.is_none() {
            return ExecutionStrategy::VectorFirst;
        }

        let estimator = CostEstimator::new(stats);
        let filter_selectivity = estimate_filter_selectivity(stats, filter);
        let filter_cost = filter.map_or(Cost::new(0.0, 0.0), |f| estimator.estimate_filter_cost(f));
        let vector_cost = estimator.estimate_hnsw_search_cost(k.max(1));
        let total_rows = stats.total_points.max(stats.row_count).max(1) as f64; // usize→f64: planning heuristic

        // Vector-first evaluates metadata predicates on over-fetched ANN candidates.
        // Required over-fetch scales inversely with filter selectivity.
        let over_fetch = (1.0 / filter_selectivity).clamp(1.0, 64.0);
        let candidate_rows = ((k.max(1) as f64) * over_fetch).min(total_rows); // usize→f64: planning heuristic
        let candidate_filter_cost = Cost::new(
            candidate_rows * 0.2, // filter scan I/O weight (same as CostEstimator)
            candidate_rows * 0.8, // filter scan CPU weight (same as CostEstimator)
        );
        let vector_first =
            vector_cost.total() + (candidate_filter_cost.total() * VECTOR_FIRST_FILTER_PENALTY);
        let graph_first = filter_cost.total()
            + (vector_cost.total() * filter_cost.io_cost.max(1.0) / GRAPH_TO_VECTOR_SCALING);
        let parallel = vector_cost.total().max(filter_cost.total()) + PARALLEL_MERGE_OVERHEAD;

        let candidates = [
            (ExecutionStrategy::VectorFirst, vector_first),
            (ExecutionStrategy::GraphFirst, graph_first),
            (ExecutionStrategy::Parallel, parallel),
        ];

        candidates
            .into_iter()
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map_or(ExecutionStrategy::Parallel, |(strategy, _)| strategy)
    }

    /// Like `choose_strategy_with_cbo` but also returns the CBO-computed over-fetch factor.
    ///
    /// Returns `(strategy, over_fetch)` where `over_fetch` is a multiplier for `k` when
    /// doing vector-first search with a selective metadata filter:
    /// - `VectorFirst` with filter: `(1 / selectivity).clamp(1, 64)`
    /// - `VectorFirst` without filter: `1`
    /// - `GraphFirst`: `1` (graph pre-filters, no over-fetch needed)
    /// - `Parallel`: `2` (merge overhead)
    #[must_use]
    pub fn choose_strategy_with_cbo_and_overfetch(
        &self,
        stats: &CollectionStats,
        filter: Option<&Condition>,
        k: usize,
    ) -> (ExecutionStrategy, usize) {
        if filter.is_none() {
            return (ExecutionStrategy::VectorFirst, 1);
        }

        let filter_selectivity = estimate_filter_selectivity(stats, filter);
        let strategy = self.choose_strategy_with_cbo(stats, filter, k);

        // Derive over-fetch from the CBO selectivity estimate.
        // GraphFirst/Parallel: fixed factors. VectorFirst: scale inversely with selectivity.
        let over_fetch = match strategy {
            ExecutionStrategy::VectorFirst => {
                // (1 / selectivity) rounded up, clamped to [1, 64].
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                // Reason: over_fetch_f is in [1.0, 64.0]; casting to usize is safe.
                let over_fetch_f = (1.0 / filter_selectivity).clamp(1.0, 64.0);
                over_fetch_f.ceil() as usize
            }
            ExecutionStrategy::GraphFirst => 1,
            ExecutionStrategy::Parallel => 2,
        };

        (strategy, over_fetch)
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
            label_count as f64 / total_nodes as f64 // u64→f64: selectivity ratio heuristic
        } else {
            1.0
        };

        let rel_sel = if total_edges == 0 {
            // No edges in graph → relationship predicate is vacuously true
            1.0
        } else if rel_type_count == 0 {
            // Edges exist but none match requested type → nothing matches
            0.0
        } else {
            rel_type_count as f64 / total_edges as f64 // u64→f64: selectivity ratio heuristic
        };

        // Combined selectivity (multiplicative for AND)
        label_sel * rel_sel
    }

    /// Choose optimal strategy for hybrid queries with ORDER BY similarity().
    ///
    /// When ORDER BY similarity() is present, we optimize for:
    /// 1. Always execute vector search first (it naturally orders by similarity)
    /// 2. Apply filters as post-processing to preserve ordering
    /// 3. Use early termination when LIMIT is specified
    ///
    /// # Arguments
    /// * `has_order_by_similarity` - True if ORDER BY similarity() is in query
    /// * `has_filter` - True if WHERE clause with non-vector conditions
    /// * `limit` - Optional LIMIT value for early termination optimization
    /// * `estimated_selectivity` - Optional estimated filter selectivity
    #[must_use]
    pub fn choose_hybrid_strategy(
        &self,
        has_order_by_similarity: bool,
        has_filter: bool,
        limit: Option<u64>,
        estimated_selectivity: Option<f64>,
    ) -> HybridExecutionPlan {
        if has_order_by_similarity {
            // ORDER BY similarity() always benefits from VectorFirst
            // because HNSW naturally returns results in similarity order
            let over_fetch_factor = if has_filter {
                // Over-fetch based on selectivity to ensure LIMIT results after filtering
                let sel = estimated_selectivity.unwrap_or(0.5);
                if sel > 0.0 {
                    (1.0 / sel).clamp(2.0, 10.0)
                } else {
                    10.0
                }
            } else {
                1.0
            };

            HybridExecutionPlan {
                strategy: ExecutionStrategy::VectorFirst,
                over_fetch_factor,
                use_early_termination: limit.is_some(),
                recompute_scores: false,
            }
        } else if has_filter {
            // No ORDER BY similarity - use standard planning
            let selectivity =
                estimated_selectivity.unwrap_or_else(|| self.stats.graph_selectivity());
            let strategy = self.choose_strategy(Some(selectivity));

            HybridExecutionPlan {
                strategy,
                over_fetch_factor: if matches!(strategy, ExecutionStrategy::VectorFirst) {
                    2.0
                } else {
                    1.0
                },
                use_early_termination: limit.is_some(),
                recompute_scores: true,
            }
        } else {
            // No filter, no ORDER BY - simple vector search
            HybridExecutionPlan {
                strategy: ExecutionStrategy::VectorFirst,
                over_fetch_factor: 1.0,
                use_early_termination: true,
                recompute_scores: false,
            }
        }
    }

    /// Estimate cost in microseconds for a given execution plan.
    ///
    /// Uses runtime statistics to estimate total query cost.
    #[must_use]
    pub fn estimate_cost(&self, plan: &HybridExecutionPlan, candidate_count: u64) -> u64 {
        let vector_cost = self.stats.avg_vector_latency_us();
        let graph_cost = self.stats.avg_graph_latency_us();

        match plan.strategy {
            ExecutionStrategy::VectorFirst => {
                // Vector search + optional filter pass
                vector_cost + candidate_count // 1µs per filter check
            }
            ExecutionStrategy::GraphFirst => {
                // Graph traversal + vector search on candidates
                graph_cost + (candidate_count * vector_cost / 1000).max(1)
            }
            ExecutionStrategy::Parallel => {
                // Max of both (parallel execution)
                vector_cost.max(graph_cost) + 10 // 10µs merge overhead
            }
        }
    }
}

/// Execution plan for hybrid queries (US-009).
#[derive(Debug, Clone, PartialEq)]
pub struct HybridExecutionPlan {
    /// Primary execution strategy.
    pub strategy: ExecutionStrategy,
    /// Factor to multiply LIMIT for over-fetching when filtering.
    pub over_fetch_factor: f64,
    /// Whether to use early termination optimization.
    pub use_early_termination: bool,
    /// Whether scores need to be recomputed after filtering.
    pub recompute_scores: bool,
}

impl Default for HybridExecutionPlan {
    fn default() -> Self {
        Self {
            strategy: ExecutionStrategy::VectorFirst,
            over_fetch_factor: 1.0,
            use_early_termination: true,
            recompute_scores: false,
        }
    }
}

/// Estimates filter selectivity from collection statistics and an optional condition.
///
/// Returns a value clamped to `[0.001, 1.0]`, defaulting to `1.0` when no filter is present.
fn estimate_filter_selectivity(stats: &CollectionStats, filter: Option<&Condition>) -> f64 {
    let estimator = CostEstimator::new(stats);
    filter.map_or(1.0, |f| {
        estimator
            .estimate_condition_selectivity(f)
            .clamp(0.001, 1.0)
    })
}

// Tests moved to planner_tests.rs per project rules
