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

use crate::collection::graph::{bfs_stream, StreamingConfig};
use crate::collection::types::Collection;
use crate::error::{Error, Result};
use crate::storage::{PayloadStorage, VectorStorage};
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

/// Parses a property path expression like "alias.property" (EPIC-058 US-007).
///
/// Returns `Some((alias, property))` if valid, `None` otherwise.
/// For nested paths like "doc.metadata.category", returns `("doc", "metadata.category")`.
#[must_use]
pub fn parse_property_path(expression: &str) -> Option<(&str, &str)> {
    // Skip special cases
    if expression == "*" || expression.contains('(') {
        return None;
    }

    // Split on first dot
    let dot_pos = expression.find('.')?;
    if dot_pos == 0 || dot_pos == expression.len() - 1 {
        return None;
    }

    let alias = &expression[..dot_pos];
    let property = &expression[dot_pos + 1..];
    Some((alias, property))
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
    pub fn execute_match(
        &self,
        match_clause: &MatchClause,
        params: &HashMap<String, serde_json::Value>,
    ) -> Result<Vec<MatchResult>> {
        // Get limit from return clause
        let limit = match_clause.return_clause.limit.map_or(100, |l| l as usize);

        // Get the first pattern
        let pattern = match_clause.patterns.first().ok_or_else(|| {
            Error::Config("MATCH query must have at least one pattern".to_string())
        })?;

        // Find start nodes
        let start_nodes = self.find_start_nodes(pattern)?;

        if start_nodes.is_empty() {
            return Ok(Vec::new());
        }

        // If no relationships in pattern, just return the start nodes
        if pattern.relationships.is_empty() {
            let mut results = Vec::new();
            // FIX: Apply WHERE filter BEFORE limit to ensure we return up to `limit` matching results
            for (node_id, bindings) in start_nodes {
                // Apply WHERE filter if present (EPIC-045 US-002)
                if let Some(ref where_clause) = match_clause.where_clause {
                    if !self.evaluate_where_condition(node_id, where_clause, params)? {
                        continue;
                    }
                }

                let mut result = MatchResult::new(node_id, 0, Vec::new());
                result.bindings.clone_from(&bindings);

                // Project properties from RETURN clause (EPIC-058 US-007)
                result.projected = self.project_properties(&bindings, &match_clause.return_clause);

                results.push(result);

                // Check limit AFTER filtering
                if results.len() >= limit {
                    break;
                }
            }
            return Ok(results);
        }

        // VP-004: Detect multi-hop patterns and route accordingly
        if pattern.relationships.len() > 1 {
            // Multi-hop: execute as chain of single-hop BFS traversals
            let start_candidates: Vec<(u64, HashMap<String, u64>)> = start_nodes;
            return self.execute_multi_hop_chain(
                pattern,
                start_candidates,
                match_clause,
                params,
                limit,
            );
        }

        // Single-hop: use existing BFS logic (no regression)
        let max_depth = self.compute_max_depth(pattern);
        let rel_types = self.extract_rel_types(pattern);

        let edge_store = self.edge_store.read();
        let mut results = Vec::new();

        for (start_id, start_bindings) in start_nodes {
            if results.len() >= limit {
                break;
            }

            let config = StreamingConfig::default()
                .with_limit(limit.saturating_sub(results.len()))
                .with_max_depth(max_depth)
                .with_rel_types(rel_types.clone());

            for traversal_result in bfs_stream(&edge_store, start_id, config) {
                if results.len() >= limit {
                    break;
                }

                let mut match_result = MatchResult::new(
                    traversal_result.target_id,
                    traversal_result.depth,
                    traversal_result.path.clone(),
                );

                match_result.bindings.clone_from(&start_bindings);

                if let Some(target_pattern) = pattern.nodes.get(traversal_result.depth as usize) {
                    if let Some(ref alias) = target_pattern.alias {
                        let alias_str: String = alias.clone();
                        match_result
                            .bindings
                            .insert(alias_str, traversal_result.target_id);
                    }
                }

                // Apply WHERE filter if present (EPIC-045 US-002)
                if let Some(ref where_clause) = match_clause.where_clause {
                    if !self.evaluate_where_condition(
                        traversal_result.target_id,
                        where_clause,
                        params,
                    )? {
                        continue;
                    }
                }

                match_result.projected =
                    self.project_properties(&match_result.bindings, &match_clause.return_clause);

                results.push(match_result);
            }
        }

        Ok(results)
    }

    /// Finds start nodes matching the first node pattern.
    fn find_start_nodes(&self, pattern: &GraphPattern) -> Result<Vec<(u64, HashMap<String, u64>)>> {
        let first_node = pattern
            .nodes
            .first()
            .ok_or_else(|| Error::Config("Pattern must have at least one node".to_string()))?;

        let mut results = Vec::new();
        let payload_storage = self.payload_storage.read();
        let vector_storage = self.vector_storage.read();

        // If node has labels, filter by label
        let has_label_filter = !first_node.labels.is_empty();
        let has_property_filter = !first_node.properties.is_empty();

        // Scan all nodes and filter
        for id in vector_storage.ids() {
            let mut matches = true;

            // Check label filter
            if has_label_filter {
                if let Ok(Some(payload)) = payload_storage.retrieve(id) {
                    if let Some(labels) = payload.get("_labels").and_then(|v| v.as_array()) {
                        let node_labels: Vec<&str> =
                            labels.iter().filter_map(|v| v.as_str()).collect();
                        for required_label in &first_node.labels {
                            let label_str: &str = required_label.as_str();
                            if !node_labels.contains(&label_str) {
                                matches = false;
                                break;
                            }
                        }
                    } else {
                        matches = false;
                    }
                } else {
                    matches = false;
                }
            }

            // Check property filter
            if matches && has_property_filter {
                if let Ok(Some(payload)) = payload_storage.retrieve(id) {
                    for (key, expected_value) in &first_node.properties {
                        if let Some(actual_value) = payload.get(key) {
                            if !Self::values_match(expected_value, actual_value) {
                                matches = false;
                                break;
                            }
                        } else {
                            matches = false;
                            break;
                        }
                    }
                } else {
                    matches = false;
                }
            }

            if matches {
                let mut bindings: HashMap<String, u64> = HashMap::new();
                if let Some(ref alias) = first_node.alias {
                    let alias_str: String = alias.clone();
                    bindings.insert(alias_str, id);
                }
                results.push((id, bindings));
            }
        }

        Ok(results)
    }

    /// Executes a multi-hop MATCH pattern as a chain of single-hop BFS traversals (VP-004).
    ///
    /// For `(a)-[:R1]->(b)-[:R2]->(c)`, this executes:
    /// 1. BFS from each start node with R1 only → yields intermediate nodes (b)
    /// 2. BFS from each intermediate node with R2 only → yields final nodes (c)
    ///
    /// Each hop binds the target node to its alias and accumulates bindings.
    /// WHERE filtering is applied after all hops complete (with full bindings).
    fn execute_multi_hop_chain(
        &self,
        pattern: &GraphPattern,
        start_candidates: Vec<(u64, HashMap<String, u64>)>,
        match_clause: &MatchClause,
        params: &HashMap<String, serde_json::Value>,
        limit: usize,
    ) -> Result<Vec<MatchResult>> {
        let edge_store = self.edge_store.read();

        // Each candidate is (current_node_id, accumulated_bindings, accumulated_path)
        let mut candidates: Vec<(u64, HashMap<String, u64>, Vec<u64>)> = start_candidates
            .into_iter()
            .map(|(id, bindings)| (id, bindings, Vec::new()))
            .collect();

        // Execute each hop in sequence
        for (hop_index, rel) in pattern.relationships.iter().enumerate() {
            let mut next_candidates: Vec<(u64, HashMap<String, u64>, Vec<u64>)> = Vec::new();

            // Determine BFS depth for this hop
            let hop_depth = rel.range.map_or(1, |(_, end)| end);

            for (current_id, bindings, path) in &candidates {
                // Configure BFS for this specific hop
                let config = StreamingConfig::default()
                    .with_max_depth(hop_depth)
                    .with_rel_types(rel.types.clone());

                for traversal_result in bfs_stream(&edge_store, *current_id, config) {
                    let mut new_bindings = bindings.clone();
                    let mut new_path = path.clone();
                    new_path.extend_from_slice(&traversal_result.path);

                    // Bind the target node to the next node pattern's alias
                    // pattern.nodes[hop_index + 1] corresponds to this hop's target
                    if let Some(target_node_pattern) = pattern.nodes.get(hop_index + 1) {
                        if let Some(ref alias) = target_node_pattern.alias {
                            new_bindings.insert(alias.clone(), traversal_result.target_id);
                        }
                    }

                    next_candidates.push((traversal_result.target_id, new_bindings, new_path));
                }
            }

            // Short-circuit: no candidates means no results possible
            if next_candidates.is_empty() {
                return Ok(Vec::new());
            }

            candidates = next_candidates;
        }

        // Build final results from the last hop's candidates
        let mut results = Vec::new();
        let total_depth = u32::try_from(pattern.relationships.len()).unwrap_or(10);

        for (node_id, bindings, path) in candidates {
            if results.len() >= limit {
                break;
            }

            // VP-004: Apply WHERE filter with binding-aware evaluation
            if let Some(ref where_clause) = match_clause.where_clause {
                if !self.evaluate_where_with_bindings(&bindings, where_clause, params)? {
                    continue;
                }
            }

            let mut match_result = MatchResult::new(node_id, total_depth, path);
            match_result.bindings = bindings;

            // Project properties from RETURN clause (EPIC-058 US-007)
            match_result.projected =
                self.project_properties(&match_result.bindings, &match_clause.return_clause);

            results.push(match_result);
        }

        Ok(results)
    }

    /// Computes maximum traversal depth from pattern.
    fn compute_max_depth(&self, pattern: &GraphPattern) -> u32 {
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
    fn extract_rel_types(&self, pattern: &GraphPattern) -> Vec<String> {
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
