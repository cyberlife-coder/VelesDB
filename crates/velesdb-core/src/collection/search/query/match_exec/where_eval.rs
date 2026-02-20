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
        bindings: Option<&HashMap<String, u64>>,
        condition: &crate::velesql::Condition,
        params: &HashMap<String, serde_json::Value>,
    ) -> Result<bool> {
        use crate::velesql::Condition;

        let payload_storage = self.payload_storage.read();

        match condition {
            Condition::Comparison(cmp) => {
                let target_id = if let Some((alias, _rest)) = cmp.column.split_once('.') {
                    bindings
                        .and_then(|b| b.get(alias).copied())
                        .unwrap_or(node_id)
                } else {
                    node_id
                };
                let Some(target_payload) = payload_storage.retrieve(target_id).ok().flatten()
                else {
                    return Ok(false);
                };

                // Get the actual value from payload.
                // Supports:
                // - simple key: age
                // - alias.property: n.age
                // - nested key path: metadata.profile.age
                let column_path = if let Some((alias, rest)) = cmp.column.split_once('.') {
                    if bindings.and_then(|b| b.get(alias)).is_some() {
                        rest
                    } else {
                        cmp.column.as_str()
                    }
                } else {
                    cmp.column.as_str()
                };

                let actual_value = Self::json_get_path(&target_payload, column_path);
                let Some(actual) = actual_value else {
                    return Ok(false);
                };

                // Resolve parameter if needed
                let resolved_value = Self::resolve_where_param(&cmp.value, params)?;

                // Compare based on operator
                Self::evaluate_comparison(cmp.operator, actual, &resolved_value)
            }
            Condition::And(left, right) => {
                let left_result = self.evaluate_where_condition(node_id, bindings, left, params)?;
                if !left_result {
                    return Ok(false);
                }
                self.evaluate_where_condition(node_id, bindings, right, params)
            }
            Condition::Or(left, right) => {
                let left_result = self.evaluate_where_condition(node_id, bindings, left, params)?;
                if left_result {
                    return Ok(true);
                }
                self.evaluate_where_condition(node_id, bindings, right, params)
            }
            Condition::Not(inner) => {
                let inner_result =
                    self.evaluate_where_condition(node_id, bindings, inner, params)?;
                Ok(!inner_result)
            }
            Condition::Group(inner) => {
                self.evaluate_where_condition(node_id, bindings, inner, params)
            }
            Condition::Similarity(sim) => {
                // EPIC-052 US-007: Evaluate similarity condition in WHERE clause
                self.evaluate_similarity_condition(node_id, sim, params)
            }
            // For other condition types (VectorSearch, VectorFusedSearch, etc.),
            // default to true as they are handled separately in execute_match_with_similarity
            _ => Ok(true),
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
                    .ok_or_else(|| Error::Config(format!("Missing vector parameter: ${name}")))?;

                // Convert JSON array to Vec<f32>
                match param_value {
                    serde_json::Value::Array(arr) => arr
                        .iter()
                        .filter_map(|v| v.as_f64().map(|f| f as f32))
                        .collect(),
                    _ => {
                        return Err(Error::Config(format!(
                            "Parameter ${name} must be a vector array"
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
        let Some(node_vector) = vector_storage.retrieve(node_id)? else {
            return Ok(false); // No vector = no match
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
                    .ok_or_else(|| Error::Config(format!("Missing parameter: ${name}")))?;

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
                            "Unsupported parameter type for ${name}: {param_value:?}",
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

    fn json_get_path<'a>(root: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
        if path.is_empty() {
            return Some(root);
        }

        let mut current = root;
        for part in path.split('.') {
            current = current.get(part)?;
        }
        Some(current)
    }
}
