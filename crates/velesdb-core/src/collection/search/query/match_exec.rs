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
        }
    }

    /// Adds a variable binding.
    #[must_use]
    pub fn with_binding(mut self, alias: String, node_id: u64) -> Self {
        self.bindings.insert(alias, node_id);
        self
    }
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
        _params: &HashMap<String, serde_json::Value>,
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
                    if !self.evaluate_where_condition(node_id, where_clause)? {
                        continue;
                    }
                }

                let mut result = MatchResult::new(node_id, 0, Vec::new());
                result.bindings = bindings;
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
                    if !self.evaluate_where_condition(traversal_result.target_id, where_clause)? {
                        continue;
                    }
                }

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
    fn evaluate_where_condition(
        &self,
        node_id: u64,
        condition: &crate::velesql::Condition,
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

                // Compare based on operator
                Self::evaluate_comparison(cmp.operator, actual, &cmp.value)
            }
            Condition::And(left, right) => {
                let left_result = self.evaluate_where_condition(node_id, left)?;
                if !left_result {
                    return Ok(false);
                }
                self.evaluate_where_condition(node_id, right)
            }
            Condition::Or(left, right) => {
                let left_result = self.evaluate_where_condition(node_id, left)?;
                if left_result {
                    return Ok(true);
                }
                self.evaluate_where_condition(node_id, right)
            }
            Condition::Not(inner) => {
                let inner_result = self.evaluate_where_condition(node_id, inner)?;
                Ok(!inner_result)
            }
            // For other condition types, default to true (not filtered)
            _ => Ok(true),
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
    /// Vector of `MatchResult` with similarity scores.
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

        // Score each result by similarity
        let vector_storage = self.vector_storage.read();
        let mut scored_results = Vec::new();

        for mut result in results {
            // Get vector for this node
            if let Ok(Some(node_vector)) = vector_storage.retrieve(result.node_id) {
                // Calculate similarity
                let similarity = metric.calculate(&node_vector, query_vector);

                // Filter by threshold (higher is better for similarity)
                if similarity >= similarity_threshold {
                    result.score = Some(similarity);
                    scored_results.push(result);
                }
            }
        }

        // Sort by similarity (descending)
        scored_results.sort_by(|a, b| b.score.unwrap_or(0.0).total_cmp(&a.score.unwrap_or(0.0)));

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_match_result_creation() {
        let result = MatchResult::new(42, 2, vec![1, 2]);
        assert_eq!(result.node_id, 42);
        assert_eq!(result.depth, 2);
        assert_eq!(result.path, vec![1, 2]);
    }

    #[test]
    fn test_match_result_with_binding() {
        let result = MatchResult::new(42, 0, vec![]).with_binding("n".to_string(), 42);
        assert_eq!(result.bindings.get("n"), Some(&42));
    }
}
