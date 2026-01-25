//! Aggregation query execution for VelesQL (EPIC-017 US-002, US-003, US-006).
//!
//! Implements streaming aggregation with O(1) memory complexity.
//! Supports GROUP BY for grouped aggregations (US-003).
//! Supports HAVING for filtering groups (US-006).
//! Supports parallel aggregation with rayon (EPIC-018 US-001).

use crate::collection::types::Collection;
use crate::error::Result;
use crate::storage::{PayloadStorage, VectorStorage};
use crate::velesql::{
    AggregateArg, AggregateFunction, AggregateResult, AggregateType, Aggregator, CompareOp,
    HavingClause, Query, SelectColumns, Value,
};
use rayon::prelude::*;
use std::collections::HashMap;

/// Maximum number of groups allowed (memory protection).
const MAX_GROUPS: usize = 10000;

/// Threshold for switching to parallel aggregation.
/// Below this, sequential is faster due to overhead.
const PARALLEL_THRESHOLD: usize = 10_000;

/// Chunk size for parallel processing.
const CHUNK_SIZE: usize = 1000;

impl Collection {
    /// Execute an aggregation query and return results as JSON.
    ///
    /// Supports COUNT(*), COUNT(column), SUM, AVG, MIN, MAX.
    /// Uses streaming aggregation - O(1) memory, single pass over data.
    ///
    /// # Arguments
    ///
    /// * `query` - Parsed VelesQL query with aggregation functions
    /// * `params` - Query parameters for placeholders
    ///
    /// # Returns
    ///
    /// JSON object with aggregation results, e.g.:
    /// ```json
    /// {"count": 100, "sum_price": 5000.0, "avg_rating": 4.5}
    /// ```
    #[allow(clippy::too_many_lines)]
    pub fn execute_aggregate(
        &self,
        query: &Query,
        _params: &HashMap<String, serde_json::Value>,
    ) -> Result<serde_json::Value> {
        let stmt = &query.select;

        // Extract aggregation functions from SELECT clause
        let aggregations: &[AggregateFunction] = match &stmt.columns {
            SelectColumns::Aggregations(aggs) => aggs,
            SelectColumns::Mixed { aggregations, .. } => aggregations,
            _ => {
                return Err(crate::error::Error::Config(
                    "execute_aggregate requires aggregation functions in SELECT".to_string(),
                ))
            }
        };

        // Check if GROUP BY is present
        if let Some(ref group_by) = stmt.group_by {
            return self.execute_grouped_aggregate(
                query,
                aggregations,
                &group_by.columns,
                stmt.having.as_ref(),
            );
        }

        // Build filter from WHERE clause if present
        let filter = stmt
            .where_clause
            .as_ref()
            .map(|cond| crate::filter::Filter::new(crate::filter::Condition::from(cond.clone())));

        // Create aggregator
        let mut aggregator = Aggregator::new();

        // Determine which columns we need to aggregate (deduplicated)
        let columns_to_aggregate: std::collections::HashSet<&str> = aggregations
            .iter()
            .filter_map(|agg| match &agg.argument {
                AggregateArg::Column(col) => Some(col.as_str()),
                AggregateArg::Wildcard => None, // COUNT(*) doesn't need column access
            })
            .collect();

        let has_count_star = aggregations
            .iter()
            .any(|agg| matches!(agg.argument, AggregateArg::Wildcard));

        // Collect all IDs for parallel processing decision
        let payload_storage = self.payload_storage.read();
        let vector_storage = self.vector_storage.read();
        let ids: Vec<u64> = vector_storage.ids();
        let total_count = ids.len();

        // Use parallel aggregation for large datasets
        let agg_result = if total_count >= PARALLEL_THRESHOLD {
            // PARALLEL: Split into chunks, aggregate each, merge results
            let columns_vec: Vec<String> = columns_to_aggregate
                .iter()
                .map(|s| (*s).to_string())
                .collect();

            let partial_aggregators: Vec<Aggregator> = ids
                .par_chunks(CHUNK_SIZE)
                .map(|chunk| {
                    let mut chunk_agg = Aggregator::new();
                    for &id in chunk {
                        let payload = payload_storage.retrieve(id).ok().flatten();

                        // Apply filter if present
                        if let Some(ref f) = filter {
                            let matches = match payload {
                                Some(ref p) => f.matches(p),
                                None => f.matches(&serde_json::Value::Null),
                            };
                            if !matches {
                                continue;
                            }
                        }

                        // Process COUNT(*)
                        if has_count_star {
                            chunk_agg.process_count();
                        }

                        // Process column aggregations
                        if let Some(ref p) = payload {
                            for col in &columns_vec {
                                if let Some(value) = Self::get_nested_value(p, col) {
                                    chunk_agg.process_value(col, value);
                                }
                            }
                        }
                    }
                    chunk_agg
                })
                .collect();

            // Merge all partial results
            let mut final_agg = Aggregator::new();
            for partial in partial_aggregators {
                final_agg.merge(partial);
            }
            final_agg.finalize()
        } else {
            // SEQUENTIAL: Original single-pass for small datasets
            for id in ids {
                let payload = payload_storage.retrieve(id).ok().flatten();

                // Apply filter if present
                if let Some(ref f) = filter {
                    let matches = match payload {
                        Some(ref p) => f.matches(p),
                        None => f.matches(&serde_json::Value::Null),
                    };
                    if !matches {
                        continue;
                    }
                }

                // Process COUNT(*)
                if has_count_star {
                    aggregator.process_count();
                }

                // Process column aggregations
                if let Some(ref p) = payload {
                    for col in &columns_to_aggregate {
                        if let Some(value) = Self::get_nested_value(p, col) {
                            aggregator.process_value(col, value);
                        }
                    }
                }
            }
            aggregator.finalize()
        };
        let mut result = serde_json::Map::new();

        // Build result based on requested aggregations
        for agg in aggregations {
            let key = if let Some(ref alias) = agg.alias {
                alias.clone()
            } else {
                match &agg.argument {
                    AggregateArg::Wildcard => "count".to_string(),
                    AggregateArg::Column(col) => {
                        let prefix = match agg.function_type {
                            AggregateType::Count => "count",
                            AggregateType::Sum => "sum",
                            AggregateType::Avg => "avg",
                            AggregateType::Min => "min",
                            AggregateType::Max => "max",
                        };
                        format!("{prefix}_{col}")
                    }
                }
            };

            let value = match (&agg.function_type, &agg.argument) {
                (AggregateType::Count, AggregateArg::Wildcard) => {
                    serde_json::json!(agg_result.count)
                }
                (AggregateType::Count, AggregateArg::Column(col)) => {
                    // COUNT(column) = number of non-null values for this column
                    let count = agg_result.counts.get(col.as_str()).copied().unwrap_or(0);
                    serde_json::json!(count)
                }
                (AggregateType::Sum, AggregateArg::Column(col)) => agg_result
                    .sums
                    .get(col.as_str())
                    .map_or(serde_json::Value::Null, |v| serde_json::json!(v)),
                (AggregateType::Avg, AggregateArg::Column(col)) => agg_result
                    .avgs
                    .get(col.as_str())
                    .map_or(serde_json::Value::Null, |v| serde_json::json!(v)),
                (AggregateType::Min, AggregateArg::Column(col)) => agg_result
                    .mins
                    .get(col.as_str())
                    .map_or(serde_json::Value::Null, |v| serde_json::json!(v)),
                (AggregateType::Max, AggregateArg::Column(col)) => agg_result
                    .maxs
                    .get(col.as_str())
                    .map_or(serde_json::Value::Null, |v| serde_json::json!(v)),
                _ => serde_json::Value::Null,
            };

            result.insert(key, value);
        }

        Ok(serde_json::Value::Object(result))
    }

    /// Get a nested value from JSON payload using dot notation.
    fn get_nested_value<'a>(
        payload: &'a serde_json::Value,
        path: &str,
    ) -> Option<&'a serde_json::Value> {
        let parts: Vec<&str> = path.split('.').collect();
        let mut current = payload;

        for part in parts {
            match current {
                serde_json::Value::Object(map) => {
                    current = map.get(part)?;
                }
                _ => return None,
            }
        }

        Some(current)
    }

    /// Execute a grouped aggregation query (GROUP BY) with optional HAVING filter.
    #[allow(clippy::too_many_lines)]
    fn execute_grouped_aggregate(
        &self,
        query: &Query,
        aggregations: &[AggregateFunction],
        group_by_columns: &[String],
        having: Option<&HavingClause>,
    ) -> Result<serde_json::Value> {
        let stmt = &query.select;

        // Build filter from WHERE clause if present
        let filter = stmt
            .where_clause
            .as_ref()
            .map(|cond| crate::filter::Filter::new(crate::filter::Condition::from(cond.clone())));

        // HashMap: GroupKey (serialized as String) -> Aggregator
        let mut groups: HashMap<String, Aggregator> = HashMap::new();

        // Determine which columns we need to aggregate
        let columns_to_aggregate: std::collections::HashSet<&str> = aggregations
            .iter()
            .filter_map(|agg| match &agg.argument {
                AggregateArg::Column(col) => Some(col.as_str()),
                AggregateArg::Wildcard => None,
            })
            .collect();

        let has_count_star = aggregations
            .iter()
            .any(|agg| matches!(agg.argument, AggregateArg::Wildcard));

        // Stream through all points
        let payload_storage = self.payload_storage.read();
        let vector_storage = self.vector_storage.read();
        let ids = vector_storage.ids();

        for id in ids {
            let payload = payload_storage.retrieve(id).ok().flatten();

            // Apply filter if present
            if let Some(ref f) = filter {
                let matches = match payload {
                    Some(ref p) => f.matches(p),
                    None => f.matches(&serde_json::Value::Null),
                };
                if !matches {
                    continue;
                }
            }

            // Extract group key from payload
            let group_key = Self::extract_group_key(payload.as_ref(), group_by_columns);

            // Check group limit
            if !groups.contains_key(&group_key) && groups.len() >= MAX_GROUPS {
                return Err(crate::error::Error::Config(format!(
                    "Too many groups (limit: {MAX_GROUPS})"
                )));
            }

            // Get or create aggregator for this group
            let aggregator = groups.entry(group_key).or_default();

            // Process COUNT(*)
            if has_count_star {
                aggregator.process_count();
            }

            // Process column aggregations
            if let Some(ref p) = payload {
                for col in &columns_to_aggregate {
                    if let Some(value) = Self::get_nested_value(p, col) {
                        aggregator.process_value(col, value);
                    }
                }
            }
        }

        // Build result array with HAVING filter
        let mut results = Vec::new();

        for (group_key, aggregator) in groups {
            let agg_result = aggregator.finalize();

            // Apply HAVING filter if present
            if let Some(having_clause) = having {
                if !Self::evaluate_having(having_clause, &agg_result) {
                    continue; // Skip groups that don't match HAVING
                }
            }

            let mut row = serde_json::Map::new();

            // Parse group key back to values and add to result
            let key_values: Vec<serde_json::Value> =
                serde_json::from_str(&group_key).unwrap_or_default();
            for (i, col_name) in group_by_columns.iter().enumerate() {
                if let Some(val) = key_values.get(i) {
                    row.insert(col_name.clone(), val.clone());
                }
            }

            // Add aggregation results
            for agg in aggregations {
                let key = if let Some(ref alias) = agg.alias {
                    alias.clone()
                } else {
                    match &agg.argument {
                        AggregateArg::Wildcard => "count".to_string(),
                        AggregateArg::Column(col) => {
                            let prefix = match agg.function_type {
                                AggregateType::Count => "count",
                                AggregateType::Sum => "sum",
                                AggregateType::Avg => "avg",
                                AggregateType::Min => "min",
                                AggregateType::Max => "max",
                            };
                            format!("{prefix}_{col}")
                        }
                    }
                };

                let value = match (&agg.function_type, &agg.argument) {
                    (AggregateType::Count, AggregateArg::Wildcard) => {
                        serde_json::json!(agg_result.count)
                    }
                    (AggregateType::Count, AggregateArg::Column(col)) => {
                        // COUNT(column) = number of non-null values for this column
                        let count = agg_result.counts.get(col.as_str()).copied().unwrap_or(0);
                        serde_json::json!(count)
                    }
                    (AggregateType::Sum, AggregateArg::Column(col)) => agg_result
                        .sums
                        .get(col.as_str())
                        .map_or(serde_json::Value::Null, |v| serde_json::json!(v)),
                    (AggregateType::Avg, AggregateArg::Column(col)) => agg_result
                        .avgs
                        .get(col.as_str())
                        .map_or(serde_json::Value::Null, |v| serde_json::json!(v)),
                    (AggregateType::Min, AggregateArg::Column(col)) => agg_result
                        .mins
                        .get(col.as_str())
                        .map_or(serde_json::Value::Null, |v| serde_json::json!(v)),
                    (AggregateType::Max, AggregateArg::Column(col)) => agg_result
                        .maxs
                        .get(col.as_str())
                        .map_or(serde_json::Value::Null, |v| serde_json::json!(v)),
                    _ => serde_json::Value::Null,
                };

                row.insert(key, value);
            }

            results.push(serde_json::Value::Object(row));
        }

        Ok(serde_json::Value::Array(results))
    }

    /// Extract group key from payload as serialized JSON array.
    fn extract_group_key(
        payload: Option<&serde_json::Value>,
        group_by_columns: &[String],
    ) -> String {
        let values: Vec<serde_json::Value> = group_by_columns
            .iter()
            .map(|col| {
                payload
                    .and_then(|p| Self::get_nested_value(p, col).cloned())
                    .unwrap_or(serde_json::Value::Null)
            })
            .collect();
        serde_json::to_string(&values).unwrap_or_else(|_| "[]".to_string())
    }

    /// Evaluate HAVING clause against aggregation result.
    fn evaluate_having(having: &HavingClause, agg_result: &AggregateResult) -> bool {
        // All conditions must match (AND semantics)
        having.conditions.iter().all(|cond| {
            let agg_value = Self::get_aggregate_value(&cond.aggregate, agg_result);
            Self::compare_values(agg_value, cond.operator, &cond.value)
        })
    }

    /// Get aggregate value from result based on function type.
    fn get_aggregate_value(agg: &AggregateFunction, result: &AggregateResult) -> Option<f64> {
        match (&agg.function_type, &agg.argument) {
            (AggregateType::Count, AggregateArg::Wildcard) => Some(result.count as f64),
            (AggregateType::Count, AggregateArg::Column(col)) => {
                // COUNT(column) = number of non-null values for this column
                result.counts.get(col.as_str()).map(|&c| c as f64)
            }
            (AggregateType::Sum, AggregateArg::Column(col)) => {
                result.sums.get(col.as_str()).copied()
            }
            (AggregateType::Avg, AggregateArg::Column(col)) => {
                result.avgs.get(col.as_str()).copied()
            }
            (AggregateType::Min, AggregateArg::Column(col)) => {
                result.mins.get(col.as_str()).copied()
            }
            (AggregateType::Max, AggregateArg::Column(col)) => {
                result.maxs.get(col.as_str()).copied()
            }
            _ => None,
        }
    }

    /// Compare aggregate value against threshold using operator.
    fn compare_values(agg_value: Option<f64>, op: CompareOp, threshold: &Value) -> bool {
        let agg = match agg_value {
            Some(v) => v,
            None => return false,
        };

        let thresh = match threshold {
            Value::Integer(i) => *i as f64,
            Value::Float(f) => *f,
            _ => return false,
        };

        // Use relative epsilon for large values (precision loss in sums)
        // Scale epsilon by max magnitude, with floor of 1.0 for small values
        let relative_epsilon = f64::EPSILON * agg.abs().max(thresh.abs()).max(1.0);

        match op {
            CompareOp::Eq => (agg - thresh).abs() < relative_epsilon,
            CompareOp::NotEq => (agg - thresh).abs() >= relative_epsilon,
            CompareOp::Gt => agg > thresh,
            CompareOp::Gte => agg >= thresh,
            CompareOp::Lt => agg < thresh,
            CompareOp::Lte => agg <= thresh,
        }
    }
}
