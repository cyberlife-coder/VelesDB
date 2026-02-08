//! WHERE clause evaluation for MATCH queries (EPIC-045 US-002).
//!
//! Handles condition evaluation, parameter resolution, and comparison operations.

use crate::collection::types::Collection;
use crate::error::{Error, Result};
use crate::storage::{PayloadStorage, VectorStorage};
use std::collections::HashMap;

impl Collection {
    /// Evaluates a WHERE condition against a node's payload (EPIC-045 US-002).
    ///
    /// Supports basic comparisons: =, <>, <, >, <=, >=
    /// Parameters are resolved from the `params` map.
    pub(crate) fn evaluate_where_condition(
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

                // VP-002: Resolve subquery first, then parameters
                let pre_resolved = self.resolve_subquery_value(&cmp.value, params, None)?;
                let resolved_value = Self::resolve_where_param(&pre_resolved, params)?;

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
            Condition::Group(inner) => self.evaluate_where_condition(node_id, inner, params),
            Condition::Similarity(sim) => {
                // EPIC-052 US-007: Evaluate similarity condition in WHERE clause
                self.evaluate_similarity_condition(node_id, sim, params)
            }
            Condition::Like(ref lk) => {
                // VP-001: LIKE/ILIKE evaluation in MATCH WHERE
                self.evaluate_like_condition(node_id, lk)
            }
            Condition::Between(ref btw) => {
                // VP-001: BETWEEN evaluation in MATCH WHERE
                self.evaluate_between_condition(node_id, btw)
            }
            Condition::In(ref inc) => {
                // VP-001: IN evaluation in MATCH WHERE
                self.evaluate_in_condition(node_id, inc)
            }
            Condition::IsNull(ref isn) => {
                // VP-001: IS NULL / IS NOT NULL evaluation in MATCH WHERE
                self.evaluate_is_null_condition(node_id, isn)
            }
            Condition::Match(ref m) => {
                // VP-001: Full-text MATCH evaluation in MATCH WHERE
                self.evaluate_match_condition(node_id, m)
            }
            // VectorSearch and VectorFusedSearch are handled at a higher level
            // by execute_match_with_similarity — pass through as true.
            Condition::VectorSearch(_) | Condition::VectorFusedSearch(_) => Ok(true),
        }
    }

    /// Evaluates a similarity condition against a node's vector (EPIC-052 US-007).
    ///
    /// Computes the similarity between the node's vector and the query vector,
    /// then compares it against the threshold using the specified operator.
    fn evaluate_similarity_condition(
        &self,
        node_id: u64,
        sim: &crate::velesql::SimilarityCondition,
        params: &HashMap<String, serde_json::Value>,
    ) -> Result<bool> {
        use crate::velesql::VectorExpr;

        // Get query vector from parameters
        let query_vector = match &sim.vector {
            VectorExpr::Literal(v) => v.clone(),
            VectorExpr::Parameter(name) => {
                let param_value = params
                    .get(name)
                    .ok_or_else(|| Error::Config(format!("Missing vector parameter: ${}", name)))?;

                // Convert JSON array to Vec<f32>
                match param_value {
                    serde_json::Value::Array(arr) => arr
                        .iter()
                        .filter_map(|v| v.as_f64().map(|f| f as f32))
                        .collect(),
                    _ => {
                        return Err(Error::Config(format!(
                            "Parameter ${} must be a vector array",
                            name
                        )));
                    }
                }
            }
        };

        if query_vector.is_empty() {
            return Ok(false);
        }

        // Get node vector
        let vector_storage = self.vector_storage.read();
        let node_vector = match vector_storage.retrieve(node_id)? {
            Some(v) => v,
            None => return Ok(false), // No vector = no match
        };

        if node_vector.len() != query_vector.len() {
            return Ok(false); // Dimension mismatch
        }

        // Compute similarity using collection's metric
        let config = self.config.read();
        let metric = config.metric;
        let higher_is_better = metric.higher_is_better();
        drop(config);

        let score = metric.calculate(&node_vector, &query_vector);

        // Evaluate threshold comparison with metric awareness
        // For distance metrics (Euclidean, Hamming): lower = more similar
        // So "similarity > X" means "distance < X" (inverted comparison)
        #[allow(clippy::cast_possible_truncation)]
        let threshold = sim.threshold as f32;

        Ok(match sim.operator {
            crate::velesql::CompareOp::Gt => {
                if higher_is_better {
                    score > threshold
                } else {
                    score < threshold
                }
            }
            crate::velesql::CompareOp::Gte => {
                if higher_is_better {
                    score >= threshold
                } else {
                    score <= threshold
                }
            }
            crate::velesql::CompareOp::Lt => {
                if higher_is_better {
                    score < threshold
                } else {
                    score > threshold
                }
            }
            crate::velesql::CompareOp::Lte => {
                if higher_is_better {
                    score <= threshold
                } else {
                    score >= threshold
                }
            }
            crate::velesql::CompareOp::Eq => (score - threshold).abs() < f32::EPSILON,
            crate::velesql::CompareOp::NotEq => (score - threshold).abs() >= f32::EPSILON,
        })
    }

    /// Resolves a Value for WHERE clause, substituting parameters from the params map.
    ///
    /// If the value is a Parameter, looks it up in params and converts to appropriate Value type.
    /// Otherwise, returns the value unchanged.
    ///
    /// # Errors
    ///
    /// Returns an error if a required parameter is missing.
    pub(crate) fn resolve_where_param(
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
    pub(crate) fn evaluate_comparison(
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

    // ========================================================================
    // VP-004: Binding-aware WHERE evaluation for multi-hop MATCH
    // ========================================================================

    /// Evaluates a WHERE condition using bindings from multi-hop traversal (VP-004).
    ///
    /// For alias-qualified columns like `c.name`:
    /// 1. Splits on first dot → (alias, property)
    /// 2. Looks up alias in bindings → node_id
    /// 3. Fetches payload for that node → checks property
    ///
    /// For unqualified columns (no dot), falls back to checking
    /// against the last bound node (backward compatible with single-hop).
    pub(crate) fn evaluate_where_with_bindings(
        &self,
        bindings: &HashMap<String, u64>,
        condition: &crate::velesql::Condition,
        params: &HashMap<String, serde_json::Value>,
    ) -> Result<bool> {
        use crate::velesql::Condition;

        match condition {
            Condition::Comparison(cmp) => {
                // Check if column is alias-qualified (e.g., "c.name")
                let actual_value = if let Some(dot_pos) = cmp.column.find('.') {
                    let alias = &cmp.column[..dot_pos];
                    let property = &cmp.column[dot_pos + 1..];

                    // Resolve alias from bindings
                    let Some(&node_id) = bindings.get(alias) else {
                        return Ok(false);
                    };

                    let payload_storage = self.payload_storage.read();
                    let payload = payload_storage.retrieve(node_id).ok().flatten();
                    let Some(payload) = payload else {
                        return Ok(false);
                    };

                    payload.get(property).cloned()
                } else {
                    // Unqualified column: try all bindings (last match wins for compat)
                    let payload_storage = self.payload_storage.read();
                    let mut found = None;
                    for &node_id in bindings.values() {
                        if let Some(payload) = payload_storage.retrieve(node_id).ok().flatten() {
                            if let Some(val) = payload.get(&cmp.column) {
                                found = Some(val.clone());
                            }
                        }
                    }
                    found
                };

                let Some(actual) = actual_value else {
                    return Ok(false);
                };

                // Resolve subquery and parameter values
                let pre_resolved = self.resolve_subquery_value(&cmp.value, params, None)?;
                let resolved_value = Self::resolve_where_param(&pre_resolved, params)?;

                Self::evaluate_comparison(cmp.operator, &actual, &resolved_value)
            }
            Condition::And(left, right) => {
                let left_result = self.evaluate_where_with_bindings(bindings, left, params)?;
                if !left_result {
                    return Ok(false);
                }
                self.evaluate_where_with_bindings(bindings, right, params)
            }
            Condition::Or(left, right) => {
                let left_result = self.evaluate_where_with_bindings(bindings, left, params)?;
                if left_result {
                    return Ok(true);
                }
                self.evaluate_where_with_bindings(bindings, right, params)
            }
            Condition::Not(inner) => {
                let inner_result = self.evaluate_where_with_bindings(bindings, inner, params)?;
                Ok(!inner_result)
            }
            Condition::Group(inner) => self.evaluate_where_with_bindings(bindings, inner, params),
            // For conditions without alias support, delegate to last bound node
            // Reason: LIKE/BETWEEN/IN/IsNull/Match/Similarity use column names
            // directly from payload — fall back to evaluating against each bound node.
            other => {
                // Try each bound node until one matches
                for &node_id in bindings.values() {
                    if self.evaluate_where_condition(node_id, other, params)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
        }
    }

    // ========================================================================
    // VP-001: MATCH WHERE condition evaluators for LIKE, BETWEEN, IN, IsNull, Match
    // ========================================================================

    /// Evaluates a LIKE/ILIKE condition against a node's payload (VP-001).
    #[allow(clippy::unnecessary_wraps)]
    fn evaluate_like_condition(
        &self,
        node_id: u64,
        lk: &crate::velesql::LikeCondition,
    ) -> Result<bool> {
        let payload_storage = self.payload_storage.read();
        let payload = payload_storage.retrieve(node_id).ok().flatten();

        let Some(payload) = payload else {
            return Ok(false);
        };

        let Some(actual) = payload.get(&lk.column) else {
            return Ok(false);
        };

        let Some(text) = actual.as_str() else {
            return Ok(false);
        };

        Ok(like_match(text, &lk.pattern, lk.case_insensitive))
    }

    /// Evaluates a BETWEEN condition against a node's payload (VP-001).
    #[allow(clippy::unnecessary_wraps)]
    fn evaluate_between_condition(
        &self,
        node_id: u64,
        btw: &crate::velesql::BetweenCondition,
    ) -> Result<bool> {
        let payload_storage = self.payload_storage.read();
        let payload = payload_storage.retrieve(node_id).ok().flatten();

        let Some(payload) = payload else {
            return Ok(false);
        };

        let Some(actual) = payload.get(&btw.column) else {
            return Ok(false);
        };

        // Reason: BETWEEN is inclusive on both ends (SQL standard)
        let gte_low = Self::evaluate_comparison(crate::velesql::CompareOp::Gte, actual, &btw.low)?;
        let lte_high =
            Self::evaluate_comparison(crate::velesql::CompareOp::Lte, actual, &btw.high)?;

        Ok(gte_low && lte_high)
    }

    /// Evaluates an IN condition against a node's payload (VP-001).
    #[allow(clippy::unnecessary_wraps)]
    fn evaluate_in_condition(
        &self,
        node_id: u64,
        inc: &crate::velesql::InCondition,
    ) -> Result<bool> {
        let payload_storage = self.payload_storage.read();
        let payload = payload_storage.retrieve(node_id).ok().flatten();

        let Some(payload) = payload else {
            return Ok(false);
        };

        let Some(actual) = payload.get(&inc.column) else {
            return Ok(false);
        };

        // Check if actual value matches any value in the IN list
        for value in &inc.values {
            if Self::evaluate_comparison(crate::velesql::CompareOp::Eq, actual, value)? {
                return Ok(true);
            }
        }

        Ok(false)
    }

    /// Evaluates an IS NULL / IS NOT NULL condition against a node's payload (VP-001).
    #[allow(clippy::unnecessary_wraps)]
    fn evaluate_is_null_condition(
        &self,
        node_id: u64,
        isn: &crate::velesql::IsNullCondition,
    ) -> Result<bool> {
        let payload_storage = self.payload_storage.read();
        let payload = payload_storage.retrieve(node_id).ok().flatten();

        let Some(payload) = payload else {
            // No payload at all: all fields are effectively NULL
            return Ok(isn.is_null);
        };

        let field_value = payload.get(&isn.column);

        let is_null = match field_value {
            // Missing field or explicit null = NULL
            None | Some(serde_json::Value::Null) => true,
            Some(_) => false,
        };

        Ok(if isn.is_null { is_null } else { !is_null })
    }

    /// Evaluates a full-text MATCH condition against a node's payload (VP-001).
    ///
    /// Uses substring containment check (case-sensitive).
    #[allow(clippy::unnecessary_wraps)]
    fn evaluate_match_condition(
        &self,
        node_id: u64,
        m: &crate::velesql::MatchCondition,
    ) -> Result<bool> {
        let payload_storage = self.payload_storage.read();
        let payload = payload_storage.retrieve(node_id).ok().flatten();

        let Some(payload) = payload else {
            return Ok(false);
        };

        let Some(actual) = payload.get(&m.column) else {
            return Ok(false);
        };

        let Some(text) = actual.as_str() else {
            return Ok(false);
        };

        // Reason: Simple substring containment for MATCH — consistent with
        // the filter::Condition::Contains behavior used in SELECT path.
        Ok(text.contains(&m.query))
    }
}

// =============================================================================
// SQL LIKE pattern matching (VP-001)
// =============================================================================

/// SQL LIKE pattern matching implementation for MATCH WHERE evaluation.
///
/// Supports:
/// - `%` matches zero or more characters
/// - `_` matches exactly one character
fn like_match(text: &str, pattern: &str, case_insensitive: bool) -> bool {
    let (text, pattern) = if case_insensitive {
        (text.to_lowercase(), pattern.to_lowercase())
    } else {
        (text.to_string(), pattern.to_string())
    };

    like_match_impl(text.as_bytes(), pattern.as_bytes())
}

/// Recursive LIKE matching using dynamic programming approach.
fn like_match_impl(text: &[u8], pattern: &[u8]) -> bool {
    let m = text.len();
    let n = pattern.len();

    // dp[i][j] = true if text[0..i] matches pattern[0..j]
    let mut dp = vec![vec![false; n + 1]; m + 1];
    dp[0][0] = true;

    // Handle leading % in pattern
    for j in 1..=n {
        if pattern[j - 1] == b'%' {
            dp[0][j] = dp[0][j - 1];
        } else {
            break;
        }
    }

    for i in 1..=m {
        for j in 1..=n {
            match pattern[j - 1] {
                b'%' => {
                    // % matches zero chars (dp[i][j-1]) or one+ chars (dp[i-1][j])
                    dp[i][j] = dp[i][j - 1] || dp[i - 1][j];
                }
                b'_' => {
                    // _ matches exactly one character
                    dp[i][j] = dp[i - 1][j - 1];
                }
                c => {
                    // Exact character match
                    dp[i][j] = dp[i - 1][j - 1] && text[i - 1] == c;
                }
            }
        }
    }

    dp[m][n]
}
