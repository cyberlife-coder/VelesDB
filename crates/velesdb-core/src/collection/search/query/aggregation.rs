//! Aggregation query execution for VelesQL (EPIC-017 US-002).
//!
//! Implements streaming aggregation with O(1) memory complexity.

use crate::collection::types::Collection;
use crate::error::Result;
use crate::storage::{PayloadStorage, VectorStorage};
use crate::velesql::{AggregateArg, AggregateType, Aggregator, Query, SelectColumns};
use std::collections::HashMap;

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
        let aggregations = match &stmt.columns {
            SelectColumns::Aggregations(aggs) => aggs,
            _ => {
                return Err(crate::error::Error::Config(
                    "execute_aggregate requires aggregation functions in SELECT".to_string(),
                ))
            }
        };

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

        // Finalize and build result
        let agg_result = aggregator.finalize();
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
                    // COUNT(column) = number of non-null values
                    let count = agg_result
                        .sums
                        .get(col.as_str())
                        .map_or(0, |_| agg_result.count);
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
}
