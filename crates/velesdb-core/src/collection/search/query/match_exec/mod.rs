//! MATCH query execution for graph pattern matching (EPIC-045 US-002).
//!
//! This module implements the `execute_match()` method for executing
//! Cypher-like MATCH queries on VelesDB collections.

// Reason: Numeric casts in MATCH query execution are intentional:
// - u64->usize for result limits: limits are small (< 1M) and bounded
// - f64->f32 for embedding vectors: precision sufficient for similarity search
// - u32->f32 for depth scoring: depth values are small (< 1000)
// - All casts are for internal query execution, not user data validation
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]

mod similarity;
mod start_nodes;
mod vector_first;
mod where_eval;

use crate::collection::graph::{concurrent_bfs_stream, StreamingConfig};
use crate::collection::types::Collection;
use crate::error::{Error, Result};
use crate::guardrails::QueryContext;
use crate::storage::LogPayloadStorage;
use crate::velesql::{GraphPattern, MatchClause};
use std::collections::HashMap;

/// Result of a MATCH query traversal.
#[derive(Debug, Clone)]
pub struct MatchResult {
    /// Node ID that was matched.
    pub node_id: u64,
    /// Depth in the traversal (0 = start node).
    pub depth: u32,
    /// Path of edge IDs from start to this node.
    pub path: Vec<u64>,
    /// Bound variables from the pattern (alias -> node_id).
    pub bindings: HashMap<String, u64>,
    /// Similarity score if combined with vector search.
    pub score: Option<f32>,
    /// Projected properties from RETURN clause (EPIC-058 US-007).
    /// Key format: "alias.property" (e.g., "author.name").
    pub projected: HashMap<String, serde_json::Value>,
}

impl MatchResult {
    /// Creates a new match result.
    #[must_use]
    pub fn new(node_id: u64, depth: u32, path: Vec<u64>) -> Self {
        Self {
            node_id,
            depth,
            path,
            bindings: HashMap::new(),
            score: None,
            projected: HashMap::new(),
        }
    }

    /// Adds a variable binding.
    #[must_use]
    pub fn with_binding(mut self, alias: String, node_id: u64) -> Self {
        self.bindings.insert(alias, node_id);
        self
    }

    /// Adds projected properties (EPIC-058 US-007).
    #[must_use]
    pub fn with_projected(mut self, projected: HashMap<String, serde_json::Value>) -> Self {
        self.projected = projected;
        self
    }
}

/// A parsed RETURN clause projection item (Fix #489).
///
/// Replaces the former `parse_property_path()` that silently returned `None`
/// for wildcards, function calls, and bare aliases — leaving `projected` empty.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProjectionItem<'a> {
    /// `RETURN *` — project all properties from all bound aliases.
    Wildcard,
    /// `RETURN similarity()` — a function call expression.
    /// The inner `&str` is the function name (e.g., `"similarity"`).
    FunctionCall(&'a str),
    /// `RETURN n.name` — a dotted property path.
    PropertyPath {
        /// The alias portion (e.g., `"n"`).
        alias: &'a str,
        /// The property portion (e.g., `"name"` or `"metadata.category"`).
        property: &'a str,
    },
    /// `RETURN n` — a bare alias referring to a bound node.
    BareAlias(&'a str),
}

/// Parses a RETURN clause expression into a [`ProjectionItem`] (Fix #489).
///
/// Handles four patterns:
/// - `"*"` → [`ProjectionItem::Wildcard`]
/// - `"similarity()"` → [`ProjectionItem::FunctionCall("similarity")`]
/// - `"n.name"` → [`ProjectionItem::PropertyPath { alias: "n", property: "name" }`]
/// - `"n"` → [`ProjectionItem::BareAlias("n")`]
#[must_use]
pub fn parse_projection_item(expression: &str) -> ProjectionItem<'_> {
    if expression == "*" {
        return ProjectionItem::Wildcard;
    }

    // Function calls contain '(' — extract the name before the parenthesis.
    if let Some(paren_pos) = expression.find('(') {
        let name = &expression[..paren_pos];
        return ProjectionItem::FunctionCall(name);
    }

    // Dotted property path: split on first dot (both halves must be non-empty).
    if let Some(dot_pos) = expression.find('.') {
        let alias = &expression[..dot_pos];
        let property = &expression[dot_pos + 1..];
        if !alias.is_empty() && !property.is_empty() {
            return ProjectionItem::PropertyPath { alias, property };
        }
    }

    // Everything else is a bare alias (including edge cases like ".x" or "x.").
    ProjectionItem::BareAlias(expression)
}

/// Parses a property path expression like "alias.property" (EPIC-058 US-007).
///
/// Returns `Some((alias, property))` if valid, `None` otherwise.
/// For nested paths like "doc.metadata.category", returns `("doc", "metadata.category")`.
///
/// **Prefer [`parse_projection_item`]** for RETURN clause projection — this function
/// only handles `PropertyPath` cases and returns `None` for wildcards, function calls,
/// and bare aliases.
#[must_use]
pub fn parse_property_path(expression: &str) -> Option<(&str, &str)> {
    match parse_projection_item(expression) {
        ProjectionItem::PropertyPath { alias, property } => Some((alias, property)),
        _ => None,
    }
}

/// Context for collecting single-node pattern results (no relationships).
struct SingleNodeCtx<'a> {
    match_clause: &'a MatchClause,
    params: &'a HashMap<String, serde_json::Value>,
    payload_guard: &'a LogPayloadStorage,
    seen_pairs: &'a mut std::collections::HashSet<(u64, u64)>,
    all_results: &'a mut Vec<MatchResult>,
    limit: usize,
}

/// Mutable state carried through BFS traversal of a single pattern.
struct TraversalCtx<'a> {
    match_clause: &'a MatchClause,
    params: &'a HashMap<String, serde_json::Value>,
    payload_guard: &'a LogPayloadStorage,
    guardrail: Option<&'a QueryContext>,
    seen_pairs: &'a mut std::collections::HashSet<(u64, u64)>,
    all_results: &'a mut Vec<MatchResult>,
    limit: usize,
    iteration_count: &'a mut u32,
    reported_cardinality: &'a mut usize,
}

impl Collection {
    /// Executes a MATCH query on this collection (EPIC-045 US-002).
    ///
    /// This method performs graph pattern matching by:
    /// 1. Finding start nodes matching the first node pattern
    /// 2. Traversing relationships according to the pattern
    /// 3. Filtering results by WHERE clause conditions
    /// 4. Returning results according to RETURN clause
    ///
    /// # Arguments
    ///
    /// * `match_clause` - The parsed MATCH clause
    /// * `params` - Query parameters for resolving placeholders
    ///
    /// # Returns
    ///
    /// Vector of `MatchResult` containing matched nodes and their bindings.
    ///
    /// # Errors
    ///
    /// Returns an error if the query cannot be executed.
    /// Executes a MATCH query without guard-rail context (backward-compatible entry point).
    pub fn execute_match(
        &self,
        match_clause: &MatchClause,
        params: &HashMap<String, serde_json::Value>,
    ) -> Result<Vec<MatchResult>> {
        self.execute_match_with_context(match_clause, params, None)
    }

    /// Executes a MATCH query on this collection (EPIC-045 US-002, EPIC-048).
    ///
    /// Performs graph pattern matching: finds start nodes, traverses
    /// relationships, enforces guard-rail limits, filters by WHERE, and
    /// projects RETURN properties.
    ///
    /// Hoists `payload_storage.read()` once before the traversal loop to avoid
    /// per-node lock acquisitions. The `ConcurrentEdgeStore` manages its own
    /// internal shard locks — no outer lock is needed.
    ///
    /// # Errors
    ///
    /// Returns an error if the query cannot be executed or a guard-rail is violated.
    pub fn execute_match_with_context(
        &self,
        match_clause: &MatchClause,
        params: &HashMap<String, serde_json::Value>,
        ctx: Option<&QueryContext>,
    ) -> Result<Vec<MatchResult>> {
        if match_clause.patterns.is_empty() {
            return Err(Error::Config(
                "MATCH query must have at least one pattern".to_string(),
            ));
        }

        let limit = match_clause.return_clause.limit.map_or(100, |l| l as usize);
        let mut all_results: Vec<MatchResult> = Vec::new();
        let mut iteration_count: u32 = 0;
        let mut reported_cardinality: usize = 0;

        // Hoist payload_storage lock once for the entire query.
        let payload_guard = self.payload_storage.read();

        for pattern in &match_clause.patterns {
            if all_results.len() >= limit {
                break;
            }
            self.execute_single_pattern(
                pattern,
                match_clause,
                params,
                ctx,
                &payload_guard,
                &self.edge_store,
                limit,
                &mut all_results,
                &mut iteration_count,
                &mut reported_cardinality,
            )?;
        }

        Ok(all_results)
    }

    /// Executes a single graph pattern: finds start nodes, then dispatches to
    /// single-node collection or BFS traversal.
    #[allow(clippy::too_many_arguments)]
    fn execute_single_pattern(
        &self,
        pattern: &GraphPattern,
        match_clause: &MatchClause,
        params: &HashMap<String, serde_json::Value>,
        ctx: Option<&QueryContext>,
        payload_guard: &LogPayloadStorage,
        edge_store: &crate::collection::graph::ConcurrentEdgeStore,
        limit: usize,
        all_results: &mut Vec<MatchResult>,
        iteration_count: &mut u32,
        reported_cardinality: &mut usize,
    ) -> Result<()> {
        let start_nodes = self.find_start_nodes(pattern)?;
        if start_nodes.is_empty() {
            return Ok(());
        }

        let mut seen_pairs: std::collections::HashSet<(u64, u64)> =
            std::collections::HashSet::new();

        if pattern.relationships.is_empty() {
            let mut sn_ctx = SingleNodeCtx {
                match_clause,
                params,
                payload_guard,
                seen_pairs: &mut seen_pairs,
                all_results,
                limit,
            };
            return self.collect_single_node_results(&start_nodes, &mut sn_ctx);
        }

        let mut trav_ctx = TraversalCtx {
            match_clause,
            params,
            payload_guard,
            guardrail: ctx,
            seen_pairs: &mut seen_pairs,
            all_results,
            limit,
            iteration_count,
            reported_cardinality,
        };
        self.traverse_pattern(pattern, &start_nodes, edge_store, &mut trav_ctx)
    }

    /// Traverses a single graph pattern via BFS for each start node.
    fn traverse_pattern(
        &self,
        pattern: &GraphPattern,
        start_nodes: &[(u64, HashMap<String, u64>)],
        edge_store: &crate::collection::graph::ConcurrentEdgeStore,
        ctx: &mut TraversalCtx<'_>,
    ) -> Result<()> {
        let max_depth = Self::compute_max_depth(pattern);
        let rel_types = Self::extract_rel_types(pattern);

        for (start_id, start_bindings) in start_nodes {
            if ctx.all_results.len() >= ctx.limit {
                break;
            }

            let config = StreamingConfig::default()
                .with_limit(ctx.limit.saturating_sub(ctx.all_results.len()))
                .with_max_depth(max_depth)
                .with_rel_types(rel_types.clone());

            for traversal_result in concurrent_bfs_stream(edge_store, *start_id, config) {
                if ctx.all_results.len() >= ctx.limit {
                    break;
                }

                *ctx.iteration_count += 1;
                self.check_periodic_guardrails(
                    ctx.guardrail,
                    *ctx.iteration_count,
                    ctx.all_results,
                    ctx.reported_cardinality,
                )?;

                self.accept_traversal_hit(
                    *start_id,
                    &traversal_result,
                    start_bindings,
                    pattern,
                    ctx,
                )?;
            }
        }
        Ok(())
    }

    /// Evaluates a single BFS hit: guard-rails, WHERE filter, dedup, and projection.
    ///
    /// Uses the pre-acquired `payload_guard` from the traversal context
    /// to avoid per-node lock acquisitions.
    fn accept_traversal_hit(
        &self,
        start_id: u64,
        traversal_result: &crate::collection::graph::TraversalResult,
        start_bindings: &HashMap<String, u64>,
        pattern: &GraphPattern,
        ctx: &mut TraversalCtx<'_>,
    ) -> Result<()> {
        let match_result = self.build_traversal_match_result(
            traversal_result,
            start_bindings,
            pattern,
            ctx.guardrail,
        )?;

        if let Some(ref where_clause) = ctx.match_clause.where_clause {
            if !self.evaluate_where_condition(
                traversal_result.target_id,
                Some(&match_result.bindings),
                where_clause,
                ctx.params,
                ctx.payload_guard,
            )? {
                return Ok(());
            }
        }

        let pair = (start_id, traversal_result.target_id);
        if !ctx.seen_pairs.insert(pair) {
            return Ok(());
        }

        let mut final_result = match_result;
        final_result.projected = self.project_properties(
            &final_result.bindings,
            &ctx.match_clause.return_clause,
            ctx.payload_guard,
        );

        ctx.all_results.push(final_result);
        Ok(())
    }

    /// Collects results for single-node patterns (no relationships).
    ///
    /// Uses the pre-acquired `payload_guard` from the context to avoid
    /// per-node lock acquisitions.
    fn collect_single_node_results(
        &self,
        start_nodes: &[(u64, HashMap<String, u64>)],
        ctx: &mut SingleNodeCtx<'_>,
    ) -> Result<()> {
        for (node_id, bindings) in start_nodes {
            if ctx.all_results.len() >= ctx.limit {
                break;
            }
            if let Some(ref where_clause) = ctx.match_clause.where_clause {
                if !self.evaluate_where_condition(
                    *node_id,
                    Some(bindings),
                    where_clause,
                    ctx.params,
                    ctx.payload_guard,
                )? {
                    continue;
                }
            }
            if ctx.seen_pairs.contains(&(*node_id, *node_id)) {
                continue;
            }
            ctx.seen_pairs.insert((*node_id, *node_id));

            let mut result = MatchResult::new(*node_id, 0, Vec::new());
            result.bindings.clone_from(bindings);
            result.projected = self.project_properties(
                bindings,
                &ctx.match_clause.return_clause,
                ctx.payload_guard,
            );
            ctx.all_results.push(result);
        }
        Ok(())
    }

    /// Periodic guard-rail checks every 100 iterations (EPIC-048).
    #[allow(clippy::unused_self)]
    fn check_periodic_guardrails(
        &self,
        ctx: Option<&QueryContext>,
        iteration_count: u32,
        all_results: &[MatchResult],
        reported_cardinality: &mut usize,
    ) -> Result<()> {
        if iteration_count % 100 != 0 {
            return Ok(());
        }
        let Some(ctx) = ctx else { return Ok(()) };
        ctx.check_timeout()
            .map_err(|e| Error::GuardRail(e.to_string()))?;
        let new_results = all_results.len().saturating_sub(*reported_cardinality);
        if new_results > 0 {
            ctx.check_cardinality(new_results)
                .map_err(|e| Error::GuardRail(e.to_string()))?;
            *reported_cardinality = all_results.len();
        }
        Ok(())
    }

    /// Builds a `MatchResult` from a traversal result with bindings and depth check.
    #[allow(clippy::unused_self)]
    fn build_traversal_match_result(
        &self,
        traversal_result: &crate::collection::graph::TraversalResult,
        start_bindings: &HashMap<String, u64>,
        pattern: &GraphPattern,
        ctx: Option<&QueryContext>,
    ) -> Result<MatchResult> {
        let mut match_result = MatchResult::new(
            traversal_result.target_id,
            traversal_result.depth,
            traversal_result.path.clone(),
        );
        match_result.bindings.clone_from(start_bindings);

        if let Some(ctx) = ctx {
            ctx.check_depth(traversal_result.depth)
                .map_err(|e| Error::GuardRail(e.to_string()))?;
        }

        if let Some(target_pattern) = pattern.nodes.get(traversal_result.depth as usize) {
            if let Some(ref alias) = target_pattern.alias {
                match_result
                    .bindings
                    .insert(alias.clone(), traversal_result.target_id);
            }
        }

        Ok(match_result)
    }
}

// Tests moved to match_exec_tests.rs per project rules
