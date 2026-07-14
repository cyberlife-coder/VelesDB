//! MATCH Query Planner for optimal execution strategy (EPIC-045 US-006).
//!
//! This module provides cost-based query planning for MATCH queries,
//! choosing between Graph-First, Vector-First, or Parallel execution.

// Reason: Numeric casts in query planning are intentional:
// - u64->f64 for limit calculations: precision loss acceptable for estimates
// - f32->f64 for selectivity: values bounded (0.0-1.0 range)
// - usize->f64 for estimate calculations: values bounded (limit * 100 max)
// - All casts used for cost estimation, not precise calculations
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]

use crate::velesql::{Condition, MatchClause};
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
        let start_labels = Self::extract_start_labels(match_clause);
        let max_depth = Self::count_hops(match_clause);
        // A vector strategy may short-cut ONLY for a start-similarity condition,
        // and NEVER when:
        //   * WHERE/RETURN references a relationship alias — VectorFirst validates
        //     candidates without binding edge aliases, so `r.prop` would silently
        //     resolve against node payloads (audit 2026-06 F2); or
        //   * an ORDER BY targets a non-similarity key (payload field, arithmetic,
        //     aggregate) — VectorFirst's approximate-HNSW, LIMIT-bounded prefix
        //     cannot yield the global top-K, which only GraphFirst's exact label
        //     enumeration + post-sort LIMIT guarantees (backlog #1b).
        if has_similarity
            && !Self::references_relationship_alias(match_clause)
            && !order_by_needs_full_candidates(&match_clause.return_clause)
        {
            let similarity_info = Self::extract_similarity_info(match_clause.where_clause.as_ref());
            if Self::is_similarity_on_start(match_clause, similarity_info.as_ref()) {
                return Self::plan_vector_first(match_clause, stats, similarity_info);
            }
            if Self::should_use_parallel(stats, similarity_info.as_ref()) {
                return Self::plan_parallel(
                    match_clause,
                    stats,
                    similarity_info,
                    start_labels,
                    max_depth,
                );
            }
        }
        MatchExecutionStrategy::GraphFirst {
            start_labels,
            max_depth,
        }
    }

    /// Checks whether any WHERE leaf or RETURN/ORDER BY expression references
    /// a relationship alias declared by the pattern (audit 2026-06 F2).
    fn references_relationship_alias(match_clause: &MatchClause) -> bool {
        let aliases: Vec<&str> = match_clause
            .patterns
            .iter()
            .flat_map(|p| p.relationships.iter())
            .filter_map(|r| r.alias.as_deref())
            .collect();
        if aliases.is_empty() {
            return false;
        }
        let where_refs = match_clause
            .where_clause
            .as_ref()
            .is_some_and(|cond| Self::condition_references_alias(cond, &aliases));
        where_refs || Self::return_clause_references_alias(&match_clause.return_clause, &aliases)
    }

    /// Checks RETURN items and ORDER BY expressions for alias references.
    fn return_clause_references_alias(
        return_clause: &crate::velesql::ReturnClause,
        aliases: &[&str],
    ) -> bool {
        let item_refs = return_clause
            .items
            .iter()
            .any(|item| column_targets_alias(&item.expression, aliases));
        let order_refs = return_clause.order_by.as_ref().is_some_and(|items| {
            items
                .iter()
                .any(|item| order_by_expr_targets_alias(&item.expr, aliases))
        });
        item_refs || order_refs
    }

    /// Recursively checks WHERE leaves for columns targeting `aliases`.
    fn condition_references_alias(condition: &Condition, aliases: &[&str]) -> bool {
        match condition {
            Condition::Comparison(cmp) => column_targets_alias(&cmp.column, aliases),
            Condition::And(left, right) | Condition::Or(left, right) => {
                Self::condition_references_alias(left, aliases)
                    || Self::condition_references_alias(right, aliases)
            }
            Condition::Not(inner) | Condition::Group(inner) => {
                Self::condition_references_alias(inner, aliases)
            }
            other => column_of_metadata_condition(other)
                .is_some_and(|column| column_targets_alias(column, aliases)),
        }
    }

    /// Checks if the similarity condition targets the start node.
    fn is_similarity_on_start(
        match_clause: &MatchClause,
        similarity_info: Option<&(String, f32, String)>,
    ) -> bool {
        let start_alias = match_clause
            .patterns
            .first()
            .and_then(|p| p.nodes.first())
            .and_then(|n| n.alias.as_ref());
        similarity_info.is_some_and(|(alias, _, _)| Some(alias) == start_alias)
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

    /// Plans a vector-first strategy.
    fn plan_vector_first(
        match_clause: &MatchClause,
        stats: &CollectionStats,
        similarity_info: Option<(String, f32, String)>,
    ) -> MatchExecutionStrategy {
        let (alias, threshold, _) = similarity_info.unwrap_or_default();
        MatchExecutionStrategy::VectorFirst {
            similarity_alias: alias,
            top_k: Self::estimate_top_k(match_clause, stats, threshold),
            threshold,
        }
    }

    /// Plans a parallel (graph + vector) strategy.
    fn plan_parallel(
        match_clause: &MatchClause,
        stats: &CollectionStats,
        similarity_info: Option<(String, f32, String)>,
        start_labels: Vec<String>,
        max_depth: u32,
    ) -> MatchExecutionStrategy {
        let (alias, threshold, _) = similarity_info.unwrap_or_default();
        MatchExecutionStrategy::Parallel {
            graph_hint: Box::new(MatchExecutionStrategy::GraphFirst {
                start_labels,
                max_depth,
            }),
            vector_hint: Box::new(MatchExecutionStrategy::VectorFirst {
                similarity_alias: alias,
                top_k: Self::estimate_top_k(match_clause, stats, threshold),
                threshold,
            }),
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

        // Reason: limit, graph_factor and selectivity are all positive, so ceil() >= 0.
        #[allow(clippy::cast_sign_loss)]
        let estimated = (limit as f64 * graph_factor / selectivity).ceil() as usize;
        estimated.clamp(limit, limit * 100)
    }

    /// Estimate selectivity based on similarity threshold.
    /// Higher threshold = more selective.
    pub(crate) fn estimate_selectivity(threshold: f32) -> f64 {
        // Heuristic: threshold 0.9 → ~10% pass, 0.5 → ~50% pass
        (1.0 - f64::from(threshold)).max(0.01)
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

/// Checks whether a column expression (`r.prop` or bare `r`) targets one of
/// the pattern's relationship aliases (audit 2026-06 F2).
fn column_targets_alias(column: &str, aliases: &[&str]) -> bool {
    let prefix = column.split('.').next().unwrap_or(column);
    aliases.contains(&prefix)
}

/// Whether the RETURN `ORDER BY` needs the full WHERE-matching candidate set
/// rather than the similarity-ranked prefix `VectorFirst` produces.
///
/// `VectorFirst` fetches a similarity-top-K subset via approximate HNSW, so it
/// can only honor an `ORDER BY` that *is* the start-node similarity (its natural
/// output). An `ORDER BY` on a payload field, arithmetic, or aggregate requires
/// every WHERE-matching node to be ranked, which only `GraphFirst`'s exact
/// enumeration guarantees (backlog #1b). `Similarity`/`SimilarityBare` orderings
/// stay on `VectorFirst` (no perf regression for the canonical top-K).
fn order_by_needs_full_candidates(return_clause: &crate::velesql::ReturnClause) -> bool {
    use crate::velesql::OrderByExpr;
    return_clause.order_by.as_ref().is_some_and(|items| {
        items.iter().any(|item| {
            matches!(
                item.expr,
                OrderByExpr::Field(_) | OrderByExpr::Aggregate(_) | OrderByExpr::Arithmetic(_)
            )
        })
    })
}

/// Checks whether an ORDER BY expression targets one of `aliases` (used to keep
/// alias-referencing ORDER BY on the planner's VectorFirst path).
fn order_by_expr_targets_alias(expr: &crate::velesql::OrderByExpr, aliases: &[&str]) -> bool {
    use crate::velesql::OrderByExpr;
    match expr {
        OrderByExpr::Field(f) => column_targets_alias(f, aliases),
        OrderByExpr::Similarity(s) => column_targets_alias(&s.field, aliases),
        OrderByExpr::SimilarityBare | OrderByExpr::Aggregate(_) | OrderByExpr::Arithmetic(_) => {
            false
        }
    }
}

/// Extracts the column name from a metadata condition variant.
///
/// Returns `Some(&str)` for condition types that carry a `column` field.
/// Non-metadata variants return `None`. Persistence-free helper shared by the
/// MATCH planner and the collection WHERE-evaluator (relocated here in P1.4).
pub(crate) fn column_of_metadata_condition(condition: &Condition) -> Option<&str> {
    match condition {
        Condition::In(ic) => Some(&ic.column),
        Condition::Between(btw) => Some(&btw.column),
        Condition::Like(lk) => Some(&lk.column),
        Condition::IsNull(isn) => Some(&isn.column),
        Condition::Match(m) => Some(&m.column),
        Condition::ContainsText(ct) => Some(&ct.column),
        Condition::Contains(c) => Some(&c.column),
        Condition::GeoDistance(gd) => Some(&gd.column),
        Condition::GeoBbox(gb) => Some(&gb.column),
        _ => None,
    }
}

// Tests moved to match_planner_tests.rs per project rules
