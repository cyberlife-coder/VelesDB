//! MATCH Query Planner for optimal execution strategy (EPIC-045 US-006).
//!
//! This module provides cost-based query planning for MATCH queries,
//! choosing between Graph-First and Vector-First execution. No plan runs both:
//! Graph-First enumerates every row the Vector-First search could return.

// Reason: Numeric casts in query planning are intentional:
// - u64->f64 for limit calculations: precision loss acceptable for estimates
// - f32->f64 for selectivity: values bounded (0.0-1.0 range)
// - usize->f64 for estimate calculations: values bounded (limit * 100 max)
// - All casts used for cost estimation, not precise calculations
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]

use crate::velesql::{CompareOp, Condition, MatchClause, OrderByExpr, SimilarityCondition};
use serde::{Deserialize, Serialize};

/// Execution strategy for MATCH queries.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
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
}

impl Default for MatchExecutionStrategy {
    fn default() -> Self {
        Self::GraphFirst {
            start_labels: Vec::new(),
            max_depth: 10,
        }
    }
}

/// Graph-shape statistics used to choose a MATCH execution strategy.
///
/// Renamed from `CollectionStats` (5.2.0): the old name collided with the
/// tabular [`crate::collection::stats::CollectionStats`] while sharing no
/// field with it — this type describes only the graph (nodes, edges,
/// labels). Persisted inside the tabular stats as an optional view.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MatchGraphStats {
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
    pub fn plan(match_clause: &MatchClause, stats: &MatchGraphStats) -> MatchExecutionStrategy {
        if let Some(seed) = Self::seeding_similarity(match_clause) {
            let alias = seed.field.split('.').next().unwrap_or_default().to_string();
            let threshold = seed.threshold as f32;
            return MatchExecutionStrategy::VectorFirst {
                similarity_alias: alias,
                top_k: Self::estimate_top_k(match_clause, stats, threshold),
                threshold,
            };
        }
        MatchExecutionStrategy::GraphFirst {
            start_labels: Self::extract_start_labels(match_clause),
            max_depth: Self::count_hops(match_clause),
        }
    }

    /// The `similarity()` leaf a vector search may seed the MATCH walk from:
    /// `VectorFirst` only chooses the walk's start nodes, nearest first, and
    /// the walk then evaluates the whole pattern and WHERE as `GraphFirst`
    /// does, so a seeded MATCH returns what an unseeded one returns as long
    /// as the nearest start nodes are the ones its rows come from. That holds
    /// when, and only when:
    ///
    /// - the clause has one pattern and an explicit RETURN `LIMIT`: the
    ///   walk stops at the LIMIT, and the seeds grow until it is reached
    ///   (without a LIMIT every qualifying row is wanted, which seeding
    ///   cannot bound);
    /// - the leaf is a top-level conjunct of WHERE (under `AND` or groups
    ///   only, never `OR` or `NOT`), so every row satisfies it;
    /// - it asks for nodes *closer* than its threshold (`>` / `>=`, which
    ///   reads "closer" under a distance metric too) and scores the pattern's
    ///   start node, so the nearest start nodes satisfy it first;
    /// - it is WHERE's first `similarity()` leaf, the one rows are scored by;
    /// - any `ORDER BY` is that score, most similar first.
    ///
    /// `None` means the plan is `GraphFirst`.
    #[must_use]
    pub(crate) fn seeding_similarity(match_clause: &MatchClause) -> Option<&SimilarityCondition> {
        let [pattern] = match_clause.patterns.as_slice() else {
            return None;
        };
        match_clause.return_clause.limit?;
        if !orders_most_similar_first(&match_clause.return_clause) {
            return None;
        }
        let start = pattern.nodes.first()?.alias.as_deref()?;
        let condition = match_clause.where_clause.as_ref()?;
        let seed = top_level_conjuncts(condition).find_map(|leaf| match leaf {
            Condition::Similarity(sim)
                if matches!(sim.operator, CompareOp::Gt | CompareOp::Gte)
                    && sim.field.split_once('.').map(|(alias, _)| alias) == Some(start) =>
            {
                Some(sim)
            }
            _ => None,
        })?;
        first_similarity(condition)
            .is_some_and(|first| std::ptr::eq(first, seed))
            .then_some(seed)
    }

    /// Extracts start labels from the first pattern node.
    fn extract_start_labels(match_clause: &MatchClause) -> Vec<String> {
        match_clause
            .patterns
            .first()
            .and_then(|p| p.nodes.first())
            .map(|n| n.labels.clone())
            .unwrap_or_default()
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
        stats: &MatchGraphStats,
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

        // Reason: limit, graph_factor and selectivity are all positive, so ceil() >= 0.
        #[allow(clippy::cast_sign_loss)]
        let estimated = (limit as f64 * graph_factor / selectivity).ceil() as usize;
        // Saturating: LIMIT is user-supplied and uncapped here, so `limit * 100`
        // could overflow (a panic in debug, a failed `clamp` assert in release).
        estimated.clamp(limit, limit.saturating_mul(100))
    }

    /// Estimate selectivity based on similarity threshold.
    /// Higher threshold = more selective.
    pub(crate) fn estimate_selectivity(threshold: f32) -> f64 {
        // Heuristic: threshold 0.9 → ~10% pass, 0.5 → ~50% pass
        (1.0 - f64::from(threshold)).max(0.01)
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
                    "GraphFirst: Traverse from nodes with labels [{labels}], max depth {max_depth}",
                )
            }
            MatchExecutionStrategy::VectorFirst {
                similarity_alias,
                top_k,
                threshold,
            } => {
                format!(
                    "VectorFirst: Search top-{top_k} candidates for '{similarity_alias}' with threshold {threshold:.2}, then validate graph",
                )
            }
        }
    }
}

/// Whether the RETURN `ORDER BY`, if any, is the rows' similarity score,
/// most similar first — the only order the nearest seeds yield first (see
/// [`MatchQueryPlanner::seeding_similarity`]).
fn orders_most_similar_first(return_clause: &crate::velesql::ReturnClause) -> bool {
    return_clause.order_by.as_ref().is_none_or(|items| {
        items
            .iter()
            .all(|item| matches!(item.expr, OrderByExpr::SimilarityBare) && item.descending)
    })
}

/// The conjuncts of `condition`: the leaves and subtrees joined to it by
/// `AND`, through groups.
fn top_level_conjuncts(condition: &Condition) -> Box<dyn Iterator<Item = &Condition> + '_> {
    match condition {
        Condition::And(left, right) => {
            Box::new(top_level_conjuncts(left).chain(top_level_conjuncts(right)))
        }
        Condition::Group(inner) => top_level_conjuncts(inner),
        other => Box::new(std::iter::once(other)),
    }
}

/// The first `similarity()` leaf of a WHERE tree, left to right: the leaf
/// MATCH rows are scored by.
pub(crate) fn first_similarity(condition: &Condition) -> Option<&SimilarityCondition> {
    match condition {
        Condition::Similarity(sim) => Some(sim),
        Condition::And(left, right) | Condition::Or(left, right) => {
            first_similarity(left).or_else(|| first_similarity(right))
        }
        Condition::Not(inner) | Condition::Group(inner) => first_similarity(inner),
        _ => None,
    }
}

// Tests moved to match_planner_tests.rs per project rules
