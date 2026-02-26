//! Grouped aggregation query execution (GROUP BY).
//!
//! Extracted from aggregation.rs for complexity reduction (Plan 04-04).
//! HAVING evaluation, sorting, and parameter resolution are in `having.rs`.

// SAFETY: Numeric casts in aggregation are intentional:
// - All casts are for computing aggregate statistics (sum, avg, count)
// - i64->usize for group limits: limits bounded by MAX_GROUPS (1M)
// - Values bounded by result set size and field cardinality
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_sign_loss)]

use super::super::where_eval::GraphMatchEvalCache;
use crate::collection::types::Collection;
use crate::error::Result;
use crate::storage::{PayloadStorage, VectorStorage};
use crate::velesql::{
    AggregateArg, AggregateFunction, AggregateResult, AggregateType, Aggregator, HavingClause,
    Query,
};
use std::collections::HashMap;

use super::GroupKey;

impl Collection {
    /// Execute a grouped aggregation query (GROUP BY) with optional HAVING filter.
    #[allow(clippy::too_many_lines)]
    pub(crate) fn execute_grouped_aggregate(
        &self,
        query: &Query,
        aggregations: &[AggregateFunction],
        group_by_columns: &[String],
        having: Option<&HavingClause>,
        params: &HashMap<String, serde_json::Value>,
    ) -> Result<serde_json::Value> {
        let stmt = &query.select;

        // EPIC-040 US-004: Extract max_groups from WITH clause if present
        let max_groups = Self::extract_max_groups_limit(stmt.with_clause.as_ref());

        let where_clause = stmt.where_clause.as_ref();
        let use_runtime_where_eval = where_clause.is_some_and(|cond| {
            Self::condition_contains_graph_match(cond) || Self::condition_requires_vector_eval(cond)
        });
        let needs_vector_eval = where_clause.is_some_and(Self::condition_requires_vector_eval);

        // BUG-5 FIX: Build filter from WHERE clause with parameter resolution.
        // For graph/vector-aware predicates, use runtime evaluator instead.
        let filter = if use_runtime_where_eval {
            None
        } else {
            where_clause.as_ref().map(|cond| {
                let resolved = Self::resolve_condition_params(cond, params);
                crate::filter::Filter::new(crate::filter::Condition::from(resolved))
            })
        };

        // HashMap: GroupKey -> Aggregator (optimized with pre-computed hash)
        let mut groups: HashMap<GroupKey, Aggregator> = HashMap::new();

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
        let mut graph_cache = GraphMatchEvalCache::default();

        for id in ids {
            let payload = payload_storage.retrieve(id).ok().flatten();

            if use_runtime_where_eval {
                let vector = if needs_vector_eval {
                    vector_storage.retrieve(id).ok().flatten()
                } else {
                    None
                };
                if let Some(cond) = where_clause {
                    let matches = self.evaluate_where_condition_for_record(
                        cond,
                        id,
                        payload.as_ref(),
                        vector.as_deref(),
                        params,
                        stmt.from_alias.as_deref(),
                        &mut graph_cache,
                    )?;
                    if !matches {
                        continue;
                    }
                }
            } else if let Some(ref f) = filter {
                let matches = match payload {
                    Some(ref p) => f.matches(p),
                    None => f.matches(&serde_json::Value::Null),
                };
                if !matches {
                    continue;
                }
            }

            // Extract group key from payload (optimized: no JSON serialization)
            let group_key = Self::extract_group_key_fast(payload.as_ref(), group_by_columns);

            // Check group limit (configurable via WITH clause)
            if !groups.contains_key(&group_key) && groups.len() >= max_groups {
                return Err(crate::error::Error::Config(format!(
                    "Too many groups (limit: {max_groups})"
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

            // Use group key values directly (no JSON parsing needed)
            for (i, col_name) in group_by_columns.iter().enumerate() {
                if let Some(val) = group_key.values.get(i) {
                    row.insert(col_name.clone(), val.clone());
                }
            }

            // Add aggregation results
            for agg in aggregations {
                let key = Self::aggregation_result_key(agg);
                let value = Self::aggregation_result_value(agg, &agg_result);
                row.insert(key, value);
            }

            results.push(serde_json::Value::Object(row));
        }

        // BUG-3 FIX: Apply ORDER BY to grouped aggregation results
        if let Some(ref order_by) = stmt.order_by {
            Self::sort_aggregation_results(&mut results, order_by);
        }

        Ok(serde_json::Value::Array(results))
    }

    /// Compute the result key for an aggregation function.
    fn aggregation_result_key(agg: &AggregateFunction) -> String {
        if let Some(ref alias) = agg.alias {
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
        }
    }

    /// Compute the result value for an aggregation function.
    fn aggregation_result_value(
        agg: &AggregateFunction,
        agg_result: &AggregateResult,
    ) -> serde_json::Value {
        match (&agg.function_type, &agg.argument) {
            (AggregateType::Count, AggregateArg::Wildcard) => {
                serde_json::json!(agg_result.count)
            }
            (AggregateType::Count, AggregateArg::Column(col)) => {
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
        }
    }
}
