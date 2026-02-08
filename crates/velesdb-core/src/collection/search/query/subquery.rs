//! Scalar subquery execution for VelesQL (VP-002).
//!
//! This module implements the subquery executor that takes a `Subquery` AST node,
//! executes the inner `SelectStatement` against the collection, and returns a
//! scalar `Value`. Used by both MATCH WHERE and SELECT WHERE paths.

use crate::collection::types::Collection;
use crate::error::Result;
use crate::velesql::SelectColumns;
use std::collections::HashMap;

// VP-002: These methods are used by tests now and will be wired into
// MATCH WHERE (Plan 02-02) and SELECT WHERE (Plan 02-03) paths.
#[allow(dead_code)]
impl Collection {
    /// Executes a scalar subquery and returns the result as a VelesQL Value.
    ///
    /// A scalar subquery must return at most one column and one row.
    /// If no rows match, returns `Value::Null`.
    /// If multiple rows match, uses the first row (SQL standard for scalar subqueries).
    ///
    /// # Arguments
    ///
    /// * `subquery` - The parsed subquery AST node
    /// * `params` - Query parameters for resolving placeholders
    /// * `outer_row` - Optional outer row context for correlated subqueries
    ///
    /// # Errors
    ///
    /// Returns an error if the inner query cannot be executed.
    pub(crate) fn execute_scalar_subquery(
        &self,
        subquery: &crate::velesql::Subquery,
        params: &HashMap<String, serde_json::Value>,
        outer_row: Option<&serde_json::Value>,
    ) -> Result<crate::velesql::Value> {
        // VP-002: Build resolved params by merging outer row correlations
        let resolved_params = self.build_subquery_params(subquery, params, outer_row);

        // Route to aggregation or regular query path based on SELECT columns
        let is_aggregation = matches!(
            &subquery.select.columns,
            SelectColumns::Aggregations(_) | SelectColumns::Mixed { .. }
        );

        if is_aggregation {
            self.execute_aggregate_subquery(&subquery.select, &resolved_params)
        } else {
            self.execute_select_subquery(&subquery.select, &resolved_params)
        }
    }

    /// Resolves a Value that may contain a subquery into a concrete Value.
    ///
    /// If the value is `Value::Subquery`, executes it and returns the scalar result.
    /// Otherwise returns the value unchanged.
    ///
    /// # Arguments
    ///
    /// * `value` - The value to resolve
    /// * `params` - Query parameters for resolving placeholders
    /// * `outer_row` - Optional outer row context for correlated subqueries
    ///
    /// # Errors
    ///
    /// Returns an error if a subquery cannot be executed.
    pub(crate) fn resolve_subquery_value(
        &self,
        value: &crate::velesql::Value,
        params: &HashMap<String, serde_json::Value>,
        outer_row: Option<&serde_json::Value>,
    ) -> Result<crate::velesql::Value> {
        match value {
            crate::velesql::Value::Subquery(sub) => {
                self.execute_scalar_subquery(sub, params, outer_row)
            }
            other => Ok(other.clone()),
        }
    }

    /// Builds the params map for subquery execution, injecting correlated column values.
    ///
    /// For correlated subqueries, looks up `outer_column` in `outer_row` and
    /// adds it to params so the inner WHERE clause can reference it via `$outer_column`.
    fn build_subquery_params(
        &self,
        subquery: &crate::velesql::Subquery,
        params: &HashMap<String, serde_json::Value>,
        outer_row: Option<&serde_json::Value>,
    ) -> HashMap<String, serde_json::Value> {
        let mut resolved = params.clone();

        if let Some(row) = outer_row {
            for corr in &subquery.correlations {
                // Look up the outer column value in the outer row's payload
                if let Some(val) = row.get(&corr.outer_column) {
                    // Inject as parameter named after the outer_column
                    // Reason: The inner WHERE references this as $outer_column
                    resolved.insert(corr.outer_column.clone(), val.clone());
                }
            }
        }

        resolved
    }

    /// Executes an aggregation subquery (e.g., `SELECT AVG(price) FROM products`).
    ///
    /// Returns the first value from the aggregation result as a `velesql::Value`.
    fn execute_aggregate_subquery(
        &self,
        stmt: &crate::velesql::SelectStatement,
        params: &HashMap<String, serde_json::Value>,
    ) -> Result<crate::velesql::Value> {
        // Build a Query wrapping the SelectStatement
        let query = crate::velesql::Query::new_select(stmt.clone());

        // Execute aggregation
        let agg_result = self.execute_aggregate(&query, params)?;

        // Extract the first value from the aggregation result object
        // Reason: execute_aggregate returns {"avg_price": 30.0} or {"max_price": 50}
        // For scalar subquery, we need just the first (and typically only) value.
        if let serde_json::Value::Object(map) = &agg_result {
            if let Some((_key, value)) = map.iter().next() {
                return Ok(json_to_velesql_value(value));
            }
        }

        Ok(crate::velesql::Value::Null)
    }

    /// Executes a regular SELECT subquery and extracts the first row's first column.
    ///
    /// Returns `Value::Null` if no rows match.
    fn execute_select_subquery(
        &self,
        stmt: &crate::velesql::SelectStatement,
        params: &HashMap<String, serde_json::Value>,
    ) -> Result<crate::velesql::Value> {
        // Cap limit at 1 for scalar subquery (optimization)
        let mut capped_stmt = stmt.clone();
        capped_stmt.limit = Some(1);

        let query = crate::velesql::Query::new_select(capped_stmt);
        let results = self.execute_query(&query, params)?;

        // Extract first result's payload → first column value
        let Some(first) = results.first() else {
            return Ok(crate::velesql::Value::Null);
        };

        let Some(ref payload) = first.point.payload else {
            return Ok(crate::velesql::Value::Null);
        };

        // Determine which column to extract
        let column_name = match &stmt.columns {
            SelectColumns::Columns(cols) if !cols.is_empty() => cols[0].name.as_str(),
            // For SELECT *, use first key from payload
            _ => {
                if let serde_json::Value::Object(map) = payload {
                    if let Some((key, val)) = map.iter().next() {
                        // Skip internal fields like _labels
                        let target = map
                            .iter()
                            .find(|(k, _)| !k.starts_with('_'))
                            .unwrap_or((key, val));
                        return Ok(json_to_velesql_value(target.1));
                    }
                }
                return Ok(crate::velesql::Value::Null);
            }
        };

        // Look up the specific column in payload
        if let Some(value) = payload.get(column_name) {
            Ok(json_to_velesql_value(value))
        } else {
            Ok(crate::velesql::Value::Null)
        }
    }
}

/// Converts a `serde_json::Value` to a `velesql::Value` for use in comparisons.
#[allow(dead_code)] // VP-002: Used by subquery executor, will be public via Plans 02-02/02-03
fn json_to_velesql_value(json: &serde_json::Value) -> crate::velesql::Value {
    match json {
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                crate::velesql::Value::Integer(i)
            } else if let Some(f) = n.as_f64() {
                crate::velesql::Value::Float(f)
            } else {
                crate::velesql::Value::Null
            }
        }
        serde_json::Value::String(s) => crate::velesql::Value::String(s.clone()),
        serde_json::Value::Bool(b) => crate::velesql::Value::Boolean(*b),
        // Null, Array, Object all map to Value::Null
        _ => crate::velesql::Value::Null,
    }
}
