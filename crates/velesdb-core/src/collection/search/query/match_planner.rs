//! MATCH Query Planner for optimal execution strategy (EPIC-045 US-006).
//!
//! This module provides cost-based query planning for MATCH queries,
//! choosing between Graph-First, Vector-First, or Parallel execution.

#![allow(dead_code)] // Planner is used by execute_match integration

use crate::velesql::{Condition, MatchClause};
use serde::{Deserialize, Serialize};

/// Execution strategy for MATCH queries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MatchExecutionStrategy {
    /// Traverse graph first, then filter by similarity.
    /// Best when: graph is selective, or no similarity condition.
    GraphFirst {
        /// Labels to filter start nodes.
        start_labels: Vec<String>,
        /// Maximum traversal depth.
        max_depth: u32,
    },

    /// Vector search first, then validate graph paths.
    /// Best when: similarity condition on start node, graph is dense.
    VectorFirst {
        /// Alias of the node with similarity condition.
        similarity_alias: String,
        /// Top-k candidates to fetch from vector index.
        top_k: usize,
        /// Similarity threshold.
        threshold: f32,
    },

    /// Execute both strategies in parallel, then merge.
    /// Best when: both graph and vector are selective.
    Parallel {
        /// Graph-first hint.
        graph_hint: Box<MatchExecutionStrategy>,
        /// Vector-first hint.
        vector_hint: Box<MatchExecutionStrategy>,
    },
}

impl Default for MatchExecutionStrategy {
    fn default() -> Self {
        Self::GraphFirst {
            start_labels: Vec::new(),
            max_depth: 10,
        }
    }
}

/// Statistics about a collection for cost estimation.
#[derive(Debug, Clone, Default)]
pub struct CollectionStats {
    /// Total number of nodes/points.
    pub total_nodes: usize,
    /// Total number of edges.
    pub total_edges: usize,
    /// Average edges per node (density).
    pub avg_degree: f64,
    /// Number of distinct labels.
    pub label_count: usize,
    /// Estimated selectivity per label (0.0-1.0).
    pub label_selectivity: f64,
}

/// Query planner for MATCH queries.
#[derive(Debug, Default)]
pub struct MatchQueryPlanner;

impl MatchQueryPlanner {
    /// Analyze a MATCH clause and choose optimal execution strategy.
    ///
    /// # Arguments
    ///
    /// * `match_clause` - The parsed MATCH clause
    /// * `stats` - Collection statistics for cost estimation
    ///
    /// # Returns
    ///
    /// The optimal execution strategy.
    #[must_use]
    pub fn plan(match_clause: &MatchClause, stats: &CollectionStats) -> MatchExecutionStrategy {
        let has_similarity = Self::has_similarity_condition(match_clause.where_clause.as_ref());
        let similarity_info = Self::extract_similarity_info(match_clause.where_clause.as_ref());

        // Check if similarity is on start node
        let start_alias = match_clause
            .patterns
            .first()
            .and_then(|p| p.nodes.first())
            .and_then(|n| n.alias.as_ref())
            .cloned();

        let similarity_on_start = similarity_info
            .as_ref()
            .is_some_and(|(alias, _, _)| Some(alias) == start_alias.as_ref());

        // Get start labels for graph-first
        let start_labels: Vec<String> = match_clause
            .patterns
            .first()
            .map(|p| {
                p.nodes
                    .first()
                    .map(|n| n.labels.clone())
                    .unwrap_or_default()
            })
            .unwrap_or_default();

        // Count traversal depth
        let max_depth = Self::count_hops(match_clause);

        // Decision logic based on heuristics
        if has_similarity && similarity_on_start {
            // Similarity condition on start node → Vector-first is optimal
            let (alias, threshold, _) = similarity_info.unwrap_or_default();
            let top_k = Self::estimate_top_k(match_clause, stats, threshold);

            MatchExecutionStrategy::VectorFirst {
                similarity_alias: alias,
                top_k,
                threshold,
            }
        } else if !has_similarity {
            // Pure graph query → Graph-first
            MatchExecutionStrategy::GraphFirst {
                start_labels,
                max_depth,
            }
        } else if Self::should_use_parallel(stats, similarity_info.as_ref()) {
            // Both selective → Parallel execution
            let (alias, threshold, _) = similarity_info.unwrap_or_default();
            MatchExecutionStrategy::Parallel {
                graph_hint: Box::new(MatchExecutionStrategy::GraphFirst {
                    start_labels: start_labels.clone(),
                    max_depth,
                }),
                vector_hint: Box::new(MatchExecutionStrategy::VectorFirst {
                    similarity_alias: alias,
                    top_k: Self::estimate_top_k(match_clause, stats, threshold),
                    threshold,
                }),
            }
        } else {
            // Similarity on intermediate/end node → Graph-first then filter
            MatchExecutionStrategy::GraphFirst {
                start_labels,
                max_depth,
            }
        }
    }

    /// Check if WHERE clause contains a similarity condition.
    fn has_similarity_condition(where_clause: Option<&Condition>) -> bool {
        where_clause.is_some_and(Self::condition_has_similarity)
    }

    /// Recursively check if a condition contains similarity().
    fn condition_has_similarity(condition: &Condition) -> bool {
        match condition {
            Condition::Similarity(_) => true,
            Condition::And(left, right) | Condition::Or(left, right) => {
                Self::condition_has_similarity(left) || Self::condition_has_similarity(right)
            }
            Condition::Not(inner) => Self::condition_has_similarity(inner),
            _ => false,
        }
    }

    /// Extract similarity info: (alias, threshold, field).
    fn extract_similarity_info(where_clause: Option<&Condition>) -> Option<(String, f32, String)> {
        where_clause.and_then(Self::extract_from_condition)
    }

    /// Recursively extract similarity info from condition.
    fn extract_from_condition(condition: &Condition) -> Option<(String, f32, String)> {
        match condition {
            Condition::Similarity(sim) => {
                // Parse field to get alias (e.g., "doc.embedding" -> "doc")
                let alias = sim
                    .field
                    .split('.')
                    .next()
                    .unwrap_or(&sim.field)
                    .to_string();
                let field = sim
                    .field
                    .split('.')
                    .nth(1)
                    .unwrap_or("embedding")
                    .to_string();
                #[allow(clippy::cast_possible_truncation)]
                let threshold = sim.threshold as f32;
                Some((alias, threshold, field))
            }
            Condition::And(left, right) | Condition::Or(left, right) => {
                Self::extract_from_condition(left).or_else(|| Self::extract_from_condition(right))
            }
            Condition::Not(inner) => Self::extract_from_condition(inner),
            _ => None,
        }
    }

    /// Count the number of hops in the pattern.
    pub(crate) fn count_hops(match_clause: &MatchClause) -> u32 {
        match_clause.patterns.first().map_or(1, |p| {
            p.relationships
                .iter()
                .map(|r| r.range.map_or(1, |(_, max)| max))
                .sum()
        })
    }

    /// Estimate top-k based on limit and selectivity.
    fn estimate_top_k(
        match_clause: &MatchClause,
        stats: &CollectionStats,
        threshold: f32,
    ) -> usize {
        let limit = match_clause
            .return_clause
            .limit
            .and_then(|l| usize::try_from(l).ok())
            .unwrap_or(100);
        let selectivity = Self::estimate_selectivity(threshold);

        // Over-fetch to account for graph filtering
        let graph_factor = if stats.avg_degree > 0.0 {
            (1.0 / stats.label_selectivity).min(10.0)
        } else {
            2.0
        };

        let estimated = (limit as f64 * graph_factor / selectivity).ceil() as usize;
        estimated.clamp(limit, limit * 100)
    }

    /// Estimate selectivity based on similarity threshold.
    /// Higher threshold = more selective.
    pub(crate) fn estimate_selectivity(threshold: f32) -> f64 {
        // Heuristic: threshold 0.9 → ~10% pass, 0.5 → ~50% pass
        (1.0 - threshold as f64).max(0.01)
    }

    /// Decide if parallel execution is beneficial.
    fn should_use_parallel(
        stats: &CollectionStats,
        similarity_info: Option<&(String, f32, String)>,
    ) -> bool {
        // Use parallel when:
        // 1. Collection is large (>10k nodes)
        // 2. Graph is dense (avg_degree > 5)
        // 3. Similarity threshold is high (>0.8)
        let large_collection = stats.total_nodes > 10_000;
        let dense_graph = stats.avg_degree > 5.0;
        let high_threshold = similarity_info.is_some_and(|(_, t, _)| *t > 0.8);

        large_collection && dense_graph && high_threshold
    }

    /// Generate a human-readable explanation of the chosen strategy.
    #[must_use]
    pub fn explain(strategy: &MatchExecutionStrategy) -> String {
        match strategy {
            MatchExecutionStrategy::GraphFirst {
                start_labels,
                max_depth,
            } => {
                let labels = if start_labels.is_empty() {
                    "any".to_string()
                } else {
                    start_labels.join(", ")
                };
                format!(
                    "GraphFirst: Traverse from nodes with labels [{}], max depth {}",
                    labels, max_depth
                )
            }
            MatchExecutionStrategy::VectorFirst {
                similarity_alias,
                top_k,
                threshold,
            } => {
                format!(
                    "VectorFirst: Search top-{} candidates for '{}' with threshold {:.2}, then validate graph",
                    top_k, similarity_alias, threshold
                )
            }
            MatchExecutionStrategy::Parallel {
                graph_hint,
                vector_hint,
            } => {
                format!(
                    "Parallel:\n  - {}\n  - {}",
                    Self::explain(graph_hint),
                    Self::explain(vector_hint)
                )
            }
        }
    }
}

// Tests moved to match_planner_tests.rs per project rules
