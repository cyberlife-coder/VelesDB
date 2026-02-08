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

// SAFETY: Numeric casts in query planner are intentional:
// - All casts are for selectivity estimation and cost calculations
// - f64/f32 casts are for computing selectivity ratios and costs
// - Values are bounded by practical limits (cardinality, selectivity in 0-1 range)
// - Precision loss acceptable for query planning heuristics
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]

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
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            // Reason: selectivity ratio * 1_000_000 is always in [0, 1_000_000] range,
            // which fits in u64. Both matched and total are unsigned, so ratio is non-negative.
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

        let rel_sel = if total_edges == 0 {
            // No edges in graph → relationship predicate is vacuously true
            1.0
        } else if rel_type_count == 0 {
            // Edges exist but none match requested type → nothing matches
            0.0
        } else {
            rel_type_count as f64 / total_edges as f64
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

// Tests moved to planner_tests.rs per project rules
