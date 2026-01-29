//! MATCH query execution for graph pattern matching (EPIC-045 US-002).
//!
//! This module implements the `execute_match()` method for executing
//! Cypher-like MATCH queries on VelesDB collections.

use crate::collection::graph::{bfs_stream, StreamingConfig};
use crate::collection::types::Collection;
use crate::error::{Error, Result};
use crate::point::SearchResult;
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

        // Compute max depth from pattern
        let max_depth = self.compute_max_depth(pattern);

        // Get relationship type filter
        let rel_types = self.extract_rel_types(pattern);

        // Execute traversal from each start node
        let edge_store = self.edge_store.read();
        let mut results = Vec::new();

        for (start_id, start_bindings) in start_nodes {
            if results.len() >= limit {
                break;
            }

            // Configure BFS traversal
            let config = StreamingConfig::default()
                .with_limit(limit.saturating_sub(results.len()))
                .with_max_depth(max_depth)
                .with_rel_types(rel_types.clone());

            // Execute BFS from this start node
            for traversal_result in bfs_stream(&edge_store, start_id, config) {
                if results.len() >= limit {
                    break;
                }

                let mut match_result = MatchResult::new(
                    traversal_result.target_id,
                    traversal_result.depth,
                    traversal_result.path.clone(),
                );

                // Copy start bindings
                match_result.bindings.clone_from(&start_bindings);

                // Add target node binding if pattern has alias
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

                // Project properties from RETURN clause (EPIC-058 US-007)
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
            max_depth = pattern.relationships.len() as u32;
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

    /// Evaluates a WHERE condition against a node's payload (EPIC-045 US-002).
    ///
    /// Supports basic comparisons: =, <>, <, >, <=, >=
    /// Parameters are resolved from the `params` map.
    fn evaluate_where_condition(
        &self,
        node_id: u64,
        condition: &crate::velesql::Condition,
        params: &HashMap<String, serde_json::Value>,
    ) -> Result<bool> {
        use crate::velesql::Condition;

        let payload_storage = self.payload_storage.read();
        let payload = payload_storage.retrieve(node_id).ok().flatten();

        match condition {
            Condition::Comparison(cmp) => {
                let Some(ref payload) = payload else {
                    return Ok(false);
                };

                // Get the actual value from payload
                let actual_value = payload.get(&cmp.column);
                let Some(actual) = actual_value else {
                    return Ok(false);
                };

                // Resolve parameter if needed
                let resolved_value = Self::resolve_where_param(&cmp.value, params)?;

                // Compare based on operator
                Self::evaluate_comparison(cmp.operator, actual, &resolved_value)
            }
            Condition::And(left, right) => {
                let left_result = self.evaluate_where_condition(node_id, left, params)?;
                if !left_result {
                    return Ok(false);
                }
                self.evaluate_where_condition(node_id, right, params)
            }
            Condition::Or(left, right) => {
                let left_result = self.evaluate_where_condition(node_id, left, params)?;
                if left_result {
                    return Ok(true);
                }
                self.evaluate_where_condition(node_id, right, params)
            }
            Condition::Not(inner) => {
                let inner_result = self.evaluate_where_condition(node_id, inner, params)?;
                Ok(!inner_result)
            }
            // For other condition types, default to true (not filtered)
            _ => Ok(true),
        }
    }

    /// Resolves a Value for WHERE clause, substituting parameters from the params map.
    ///
    /// If the value is a Parameter, looks it up in params and converts to appropriate Value type.
    /// Otherwise, returns the value unchanged.
    ///
    /// # Errors
    ///
    /// Returns an error if a required parameter is missing.
    fn resolve_where_param(
        value: &crate::velesql::Value,
        params: &HashMap<String, serde_json::Value>,
    ) -> Result<crate::velesql::Value> {
        use crate::velesql::Value;

        match value {
            Value::Parameter(name) => {
                let param_value = params
                    .get(name)
                    .ok_or_else(|| Error::Config(format!("Missing parameter: ${}", name)))?;

                // Convert JSON value to VelesQL Value
                Ok(match param_value {
                    serde_json::Value::Number(n) => {
                        if let Some(i) = n.as_i64() {
                            Value::Integer(i)
                        } else if let Some(f) = n.as_f64() {
                            Value::Float(f)
                        } else {
                            Value::Null
                        }
                    }
                    serde_json::Value::String(s) => Value::String(s.clone()),
                    serde_json::Value::Bool(b) => Value::Boolean(*b),
                    serde_json::Value::Null => Value::Null,
                    _ => {
                        return Err(Error::Config(format!(
                            "Unsupported parameter type for ${}: {:?}",
                            name, param_value
                        )));
                    }
                })
            }
            // Non-parameter values pass through unchanged
            other => Ok(other.clone()),
        }
    }

    /// Evaluates a comparison operation.
    #[allow(clippy::unnecessary_wraps)] // Consistent with other evaluation methods
    fn evaluate_comparison(
        operator: crate::velesql::CompareOp,
        actual: &serde_json::Value,
        expected: &crate::velesql::Value,
    ) -> Result<bool> {
        use crate::velesql::{CompareOp, Value};

        match (actual, expected) {
            // Integer comparisons
            (serde_json::Value::Number(n), Value::Integer(i)) => {
                let Some(actual_i) = n.as_i64() else {
                    return Ok(false);
                };
                Ok(match operator {
                    CompareOp::Eq => actual_i == *i,
                    CompareOp::NotEq => actual_i != *i,
                    CompareOp::Lt => actual_i < *i,
                    CompareOp::Gt => actual_i > *i,
                    CompareOp::Lte => actual_i <= *i,
                    CompareOp::Gte => actual_i >= *i,
                })
            }
            // Float comparisons
            (serde_json::Value::Number(n), Value::Float(f)) => {
                let Some(actual_f) = n.as_f64() else {
                    return Ok(false);
                };
                Ok(match operator {
                    CompareOp::Eq => (actual_f - *f).abs() < 0.001,
                    CompareOp::NotEq => (actual_f - *f).abs() >= 0.001,
                    CompareOp::Lt => actual_f < *f,
                    CompareOp::Gt => actual_f > *f,
                    CompareOp::Lte => actual_f <= *f,
                    CompareOp::Gte => actual_f >= *f,
                })
            }
            // String comparisons
            (serde_json::Value::String(s), Value::String(expected_s)) => Ok(match operator {
                CompareOp::Eq => s == expected_s,
                CompareOp::NotEq => s != expected_s,
                CompareOp::Lt => s < expected_s,
                CompareOp::Gt => s > expected_s,
                CompareOp::Lte => s <= expected_s,
                CompareOp::Gte => s >= expected_s,
            }),
            // Boolean comparisons
            (serde_json::Value::Bool(b), Value::Boolean(expected_b)) => Ok(match operator {
                CompareOp::Eq => b == expected_b,
                CompareOp::NotEq => b != expected_b,
                _ => false,
            }),
            // Null comparisons
            (serde_json::Value::Null, Value::Null) => Ok(matches!(operator, CompareOp::Eq)),
            (_, Value::Null) => Ok(matches!(operator, CompareOp::NotEq)),
            // Type mismatch
            _ => Ok(false),
        }
    }

    /// Projects properties from RETURN clause for a match result (EPIC-058 US-007).
    ///
    /// Resolves property paths like "author.name" by:
    /// 1. Looking up the alias in bindings to get node_id
    /// 2. Fetching the payload for that node
    /// 3. Extracting the property value
    fn project_properties(
        &self,
        bindings: &HashMap<String, u64>,
        return_clause: &crate::velesql::ReturnClause,
    ) -> HashMap<String, serde_json::Value> {
        let payload_storage = self.payload_storage.read();
        let mut projected = HashMap::new();

        for item in &return_clause.items {
            // Parse property path (e.g., "author.name" -> ("author", "name"))
            if let Some((alias, property)) = parse_property_path(&item.expression) {
                // Get node_id for this alias
                if let Some(&node_id) = bindings.get(alias) {
                    // Get payload for this node
                    if let Ok(Some(payload)) = payload_storage.retrieve(node_id) {
                        // Extract property value (support nested paths)
                        if let Some(payload_map) = payload.as_object() {
                            if let Some(value) = Self::get_nested_property(payload_map, property) {
                                let key = item
                                    .alias
                                    .clone()
                                    .unwrap_or_else(|| item.expression.clone());
                                projected.insert(key, value.clone());
                            }
                        }
                    }
                }
            }
        }

        projected
    }

    /// Gets a nested property from a JSON object (EPIC-058 US-007).
    ///
    /// Supports paths like "metadata.category" for nested access.
    /// Limited to 10 levels of nesting to prevent abuse.
    fn get_nested_property<'a>(
        payload: &'a serde_json::Map<String, serde_json::Value>,
        path: &str,
    ) -> Option<&'a serde_json::Value> {
        // Limit nesting depth to prevent potential abuse
        const MAX_NESTING_DEPTH: usize = 10;

        let parts: Vec<&str> = path.split('.').collect();

        // Bounds check on nesting depth
        if parts.len() > MAX_NESTING_DEPTH {
            tracing::warn!(
                "Property path '{}' exceeds max nesting depth of {}",
                path,
                MAX_NESTING_DEPTH
            );
            return None;
        }

        let first_key = *parts.first()?;
        let mut current: &serde_json::Value = payload.get(first_key)?;

        for part in parts.iter().skip(1) {
            current = current.as_object()?.get(*part)?;
        }

        Some(current)
    }

    /// Executes a MATCH query with similarity scoring (EPIC-045 US-003).
    ///
    /// This method combines graph pattern matching with vector similarity,
    /// enabling hybrid queries like:
    /// `MATCH (n:Article)-[:CITED]->(m) WHERE similarity(m.embedding, $query) > 0.8 RETURN m`
    ///
    /// # Arguments
    ///
    /// * `match_clause` - The parsed MATCH clause
    /// * `query_vector` - The query vector for similarity scoring
    /// * `similarity_threshold` - Minimum similarity score (0.0 to 1.0)
    /// * `params` - Query parameters
    ///
    /// # Returns
    ///
    /// Vector of `MatchResult` with similarity scores and projected properties.
    pub fn execute_match_with_similarity(
        &self,
        match_clause: &MatchClause,
        query_vector: &[f32],
        similarity_threshold: f32,
        params: &HashMap<String, serde_json::Value>,
    ) -> Result<Vec<MatchResult>> {
        // First, execute the basic MATCH query
        let results = self.execute_match(match_clause, params)?;

        if results.is_empty() {
            return Ok(results);
        }

        // Get the metric from config
        let config = self.config.read();
        let metric = config.metric;
        drop(config);

        // Score each result by similarity/distance
        let vector_storage = self.vector_storage.read();
        let mut scored_results = Vec::new();
        let higher_is_better = metric.higher_is_better();

        for mut result in results {
            // Get vector for this node
            if let Ok(Some(node_vector)) = vector_storage.retrieve(result.node_id) {
                // Calculate similarity/distance
                let score = metric.calculate(&node_vector, query_vector);

                // Filter by threshold - metric-aware comparison
                // For similarity metrics (Cosine, DotProduct, Jaccard): higher >= threshold
                // For distance metrics (Euclidean, Hamming): lower <= threshold
                let passes_threshold = if higher_is_better {
                    score >= similarity_threshold
                } else {
                    score <= similarity_threshold
                };

                if passes_threshold {
                    result.score = Some(score);

                    // Project properties from RETURN clause (EPIC-058 US-007)
                    result.projected =
                        self.project_properties(&result.bindings, &match_clause.return_clause);

                    scored_results.push(result);
                }
            }
        }

        // Sort by score - metric-aware ordering
        // For similarity: descending (higher = more similar)
        // For distance: ascending (lower = more similar)
        if higher_is_better {
            scored_results
                .sort_by(|a, b| b.score.unwrap_or(0.0).total_cmp(&a.score.unwrap_or(0.0)));
        } else {
            scored_results.sort_by(|a, b| {
                a.score
                    .unwrap_or(f32::MAX)
                    .total_cmp(&b.score.unwrap_or(f32::MAX))
            });
        }

        Ok(scored_results)
    }

    /// Applies ORDER BY to match results (EPIC-045 US-005).
    ///
    /// Supports ordering by:
    /// - `similarity()` - Vector similarity score
    /// - Property path (e.g., `n.name`)
    /// - Depth
    pub fn order_match_results(results: &mut [MatchResult], order_by: &str, descending: bool) {
        match order_by {
            "similarity()" | "similarity" => {
                results.sort_by(|a, b| {
                    let cmp = a.score.unwrap_or(0.0).total_cmp(&b.score.unwrap_or(0.0));
                    if descending {
                        cmp.reverse()
                    } else {
                        cmp
                    }
                });
            }
            "depth" => {
                results.sort_by(|a, b| {
                    let cmp = a.depth.cmp(&b.depth);
                    if descending {
                        cmp.reverse()
                    } else {
                        cmp
                    }
                });
            }
            _ => {
                // For property paths, we need payload access
                // This is a TODO for future enhancement
            }
        }
    }

    /// Converts MatchResults to SearchResults for unified API (EPIC-045 US-002).
    ///
    /// This allows MATCH queries to return the same result type as SELECT queries,
    /// enabling consistent downstream processing.
    pub fn match_results_to_search_results(
        &self,
        match_results: Vec<MatchResult>,
    ) -> Result<Vec<SearchResult>> {
        let payload_storage = self.payload_storage.read();
        let vector_storage = self.vector_storage.read();

        let mut results = Vec::new();

        for mr in match_results {
            // Get vector and payload for the node
            let vector = vector_storage
                .retrieve(mr.node_id)?
                .unwrap_or_else(Vec::new);
            let payload = payload_storage.retrieve(mr.node_id).ok().flatten();

            let point = crate::Point {
                id: mr.node_id,
                vector,
                payload,
            };

            // Use depth as inverse score (closer = higher score)
            let score = mr.score.unwrap_or(1.0 / (mr.depth as f32 + 1.0));

            results.push(SearchResult::new(point, score));
        }

        Ok(results)
    }
}

// Tests moved to match_exec_tests.rs per project rules
