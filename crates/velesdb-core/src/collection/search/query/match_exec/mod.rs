//! MATCH query execution for graph pattern matching (EPIC-045 US-002).
//!
//! This module implements the `execute_match()` method for executing
//! Cypher-like MATCH queries on VelesDB collections.

// SAFETY: Numeric casts in MATCH query execution are intentional:
// - u64->usize for result limits: limits are small (< 1M) and bounded
// - f64->f32 for embedding vectors: precision sufficient for similarity search
// - u32->f32 for depth scoring: depth values are small (< 1000)
// - All casts are for internal query execution, not user data validation
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]

mod similarity;
mod where_eval;

use crate::collection::graph::{concurrent_bfs_stream, StreamingConfig};
use crate::collection::types::Collection;
use crate::error::{Error, Result};
use crate::guardrails::QueryContext;
use crate::storage::{LogPayloadStorage, PayloadStorage, VectorStorage};
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

    /// Finds start nodes matching the first node pattern.
    ///
    /// When the pattern specifies labels (e.g., `(n:Person)`), uses the
    /// `LabelIndex` bitmap intersection for O(k) lookup instead of scanning
    /// all N nodes. Falls back to full scan when no labels are specified
    /// or when the label index contains large IDs that could not be indexed
    /// in the `RoaringBitmap` (u32 limitation).
    ///
    /// # Lock note
    ///
    /// The caller (`execute_match_with_context`) already holds a
    /// `payload_storage.read()` guard. This method (and its delegates
    /// `find_start_nodes_indexed`, `find_start_nodes_full_scan`) may acquire
    /// a second concurrent read lock on the same `payload_storage`. This is
    /// safe because `parking_lot::RwLock` allows unlimited concurrent readers
    /// with no poisoning or deadlock risk. Refactoring to pass the existing
    /// guard down would require changing 4+ function signatures for minimal
    /// runtime benefit (read locks are non-blocking).
    fn find_start_nodes(&self, pattern: &GraphPattern) -> Result<Vec<(u64, HashMap<String, u64>)>> {
        let first_node = pattern
            .nodes
            .first()
            .ok_or_else(|| Error::Config("Pattern must have at least one node".to_string()))?;

        // Fast path: use label index when labels are specified.
        if !first_node.labels.is_empty() {
            let has_large = self.label_index.read().has_large_ids();
            let indexed = self.find_start_nodes_indexed(first_node);

            if has_large {
                // RoaringBitmap cannot index IDs > u32::MAX, so the bitmap
                // result may be incomplete. Fall back to a full scan and
                // merge the results to avoid silently missing large-ID nodes.
                return Ok(self.merge_with_full_scan(indexed, first_node));
            }
            return Ok(indexed);
        }

        // Slow path: full scan (no labels in pattern).
        Ok(self.find_start_nodes_full_scan(first_node))
    }

    /// Merges indexed bitmap results with a full scan to capture nodes whose
    /// IDs exceed `u32::MAX` (not representable in `RoaringBitmap`).
    ///
    /// Collects IDs already present in `indexed` into a set, then appends
    /// any full-scan results that are not already covered.
    fn merge_with_full_scan(
        &self,
        indexed: Vec<(u64, HashMap<String, u64>)>,
        first_node: &crate::velesql::NodePattern,
    ) -> Vec<(u64, HashMap<String, u64>)> {
        let full = self.find_start_nodes_full_scan(first_node);
        if indexed.is_empty() {
            return full;
        }
        let existing: std::collections::HashSet<u64> = indexed.iter().map(|(id, _)| *id).collect();
        let mut merged = indexed;
        for entry in full {
            if !existing.contains(&entry.0) {
                merged.push(entry);
            }
        }
        merged
    }

    /// O(k) label-indexed lookup for start nodes.
    ///
    /// Intersects label bitmaps from `LabelIndex`, then filters by properties
    /// if needed. Only reads payload storage for nodes that pass the label
    /// filter, avoiding the O(N) full scan.
    fn find_start_nodes_indexed(
        &self,
        first_node: &crate::velesql::NodePattern,
    ) -> Vec<(u64, HashMap<String, u64>)> {
        let label_idx = self.label_index.read();
        let bitmap = label_idx.lookup_intersection(&first_node.labels);
        drop(label_idx);

        let Some(bitmap) = bitmap else {
            // No nodes match the required labels.
            return Vec::new();
        };

        if first_node.properties.is_empty() {
            // Labels-only: every bitmap entry is a match.
            return bitmap
                .iter()
                .map(|id| Self::build_start_binding(u64::from(id), first_node))
                .collect();
        }

        // Labels + properties: filter bitmap entries by property match.
        let payload_storage = self.payload_storage.read();
        bitmap
            .iter()
            .filter(|&id| {
                Self::node_matches_properties_by_id(
                    u64::from(id),
                    &first_node.properties,
                    &payload_storage,
                )
            })
            .map(|id| Self::build_start_binding(u64::from(id), first_node))
            .collect()
    }

    /// O(N) full scan fallback for start nodes (no labels, or large-ID fallback).
    fn find_start_nodes_full_scan(
        &self,
        first_node: &crate::velesql::NodePattern,
    ) -> Vec<(u64, HashMap<String, u64>)> {
        let payload_storage = self.payload_storage.read();
        let vector_storage = self.vector_storage.read();
        let needs_payload = !first_node.properties.is_empty() || !first_node.labels.is_empty();

        // Union of vector IDs and payload IDs — graph-only nodes have no vector.
        let mut all_ids: std::collections::HashSet<u64> =
            vector_storage.ids().into_iter().collect();
        for id in payload_storage.ids() {
            all_ids.insert(id);
        }

        all_ids
            .into_iter()
            .filter(|id| {
                Self::node_matches_pattern(*id, first_node, needs_payload, &payload_storage)
            })
            .map(|id| Self::build_start_binding(id, first_node))
            .collect()
    }

    /// Checks if a node's payload satisfies property filters (label already verified).
    fn node_matches_properties_by_id(
        id: u64,
        properties: &HashMap<String, crate::velesql::Value>,
        payload_storage: &crate::storage::LogPayloadStorage,
    ) -> bool {
        let payload_opt = payload_storage.retrieve(id).ok().flatten();
        Self::node_matches_properties(payload_opt.as_ref(), properties)
    }

    /// Returns true if a node matches the label and property filters of a pattern.
    fn node_matches_pattern(
        id: u64,
        node: &crate::velesql::NodePattern,
        needs_payload: bool,
        payload_storage: &crate::storage::LogPayloadStorage,
    ) -> bool {
        if !needs_payload {
            return true;
        }
        let payload_opt = payload_storage.retrieve(id).ok().flatten();
        if !node.labels.is_empty() && !Self::node_matches_labels(payload_opt.as_ref(), &node.labels)
        {
            return false;
        }
        node.properties.is_empty()
            || Self::node_matches_properties(payload_opt.as_ref(), &node.properties)
    }

    /// Builds a `(node_id, bindings)` pair for a start node.
    fn build_start_binding(
        id: u64,
        node: &crate::velesql::NodePattern,
    ) -> (u64, HashMap<String, u64>) {
        let mut bindings: HashMap<String, u64> = HashMap::new();
        if let Some(ref alias) = node.alias {
            bindings.insert(alias.clone(), id);
        }
        (id, bindings)
    }

    /// Checks if a node's payload matches the required labels.
    fn node_matches_labels(payload: Option<&serde_json::Value>, required: &[String]) -> bool {
        let Some(payload) = payload else { return false };
        let Some(labels) = payload.get("_labels").and_then(|v| v.as_array()) else {
            return false;
        };
        let node_labels: Vec<&str> = labels.iter().filter_map(|v| v.as_str()).collect();
        required.iter().all(|r| node_labels.contains(&r.as_str()))
    }

    /// Checks if a node's payload matches the required properties.
    fn node_matches_properties(
        payload: Option<&serde_json::Value>,
        properties: &HashMap<String, crate::velesql::Value>,
    ) -> bool {
        let Some(payload) = payload else { return false };
        properties.iter().all(|(key, expected)| {
            payload
                .get(key)
                .is_some_and(|actual| Self::values_match(expected, actual))
        })
    }

    /// Computes maximum traversal depth from pattern.
    fn compute_max_depth(pattern: &GraphPattern) -> u32 {
        let mut max_depth = 0u32;

        for rel in &pattern.relationships {
            if let Some((_, end)) = rel.range {
                max_depth = max_depth.saturating_add(end.min(10)); // Cap at 10
            } else {
                max_depth = max_depth.saturating_add(1);
            }
        }

        // Default to at least 1 if we have relationships
        if max_depth == 0 && !pattern.relationships.is_empty() {
            // SAFETY: Pattern relationships count is typically < 10, capped at 10 anyway
            max_depth = u32::try_from(pattern.relationships.len()).unwrap_or(10);
        }

        max_depth.min(10) // Cap at 10 for safety
    }

    /// Extracts relationship type filters from pattern.
    fn extract_rel_types(pattern: &GraphPattern) -> Vec<String> {
        let mut types = Vec::new();
        for rel in &pattern.relationships {
            types.extend(rel.types.clone());
        }
        types
    }

    /// Compares a VelesQL Value with a JSON value.
    fn values_match(velesql_value: &crate::velesql::Value, json_value: &serde_json::Value) -> bool {
        use crate::velesql::Value;

        match (velesql_value, json_value) {
            (Value::String(s), serde_json::Value::String(js)) => s == js,
            (Value::Integer(i), serde_json::Value::Number(n)) => {
                n.as_i64().is_some_and(|ni| *i == ni)
            }
            (Value::UnsignedInteger(u), serde_json::Value::Number(n)) => {
                n.as_u64().is_some_and(|nu| *u == nu)
            }
            (Value::Float(f), serde_json::Value::Number(n)) => {
                n.as_f64().is_some_and(|nf| (*f - nf).abs() < 0.001)
            }
            (Value::Boolean(b), serde_json::Value::Bool(jb)) => b == jb,
            (Value::Null, serde_json::Value::Null) => true,
            _ => false,
        }
    }
}

// Tests moved to match_exec_tests.rs per project rules
