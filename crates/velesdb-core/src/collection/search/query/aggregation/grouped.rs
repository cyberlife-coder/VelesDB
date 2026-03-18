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
use super::GroupKey;
use crate::collection::types::Collection;
use crate::error::Result;
use crate::storage::{PayloadStorage, VectorStorage};
use crate::velesql::{
    AggregateArg, AggregateFunction, AggregateResult, AggregateType, Aggregator, HavingClause,
    Query,
};
use std::collections::HashMap;

impl Collection {
    /// Execute a grouped aggregation query (GROUP BY) with optional HAVING filter.
    pub(crate) fn execute_grouped_aggregate(
        &self,
        query: &Query,
        aggregations: &[AggregateFunction],
        group_by_columns: &[String],
        having: Option<&HavingClause>,
        params: &HashMap<String, serde_json::Value>,
    ) -> Result<serde_json::Value> {
        let stmt = &query.select;
        let max_groups = Self::extract_max_groups_limit(stmt.with_clause.as_ref());

        let groups =
            self.scan_and_group(stmt, aggregations, group_by_columns, max_groups, params)?;

        let results = Self::build_grouped_results(
            groups,
            aggregations,
            group_by_columns,
            having,
            stmt.order_by.as_deref(),
        );

        Ok(serde_json::Value::Array(results))
    }

    /// Scans all points, applies WHERE filter, and groups by key.
    fn scan_and_group(
        &self,
        stmt: &crate::velesql::SelectStatement,
        aggregations: &[AggregateFunction],
        group_by_columns: &[String],
        max_groups: usize,
        params: &HashMap<String, serde_json::Value>,
    ) -> Result<HashMap<GroupKey, Aggregator>> {
        let where_clause = stmt.where_clause.as_ref();
        let use_runtime = Self::needs_runtime_where_eval(where_clause);
        let needs_vector_eval = where_clause.is_some_and(Self::condition_requires_vector_eval);
        let filter = Self::build_static_filter(where_clause, use_runtime, params);
        let (columns_vec, has_count_star) = Self::prepare_agg_columns(aggregations);

        let payload_storage = self.payload_storage.read();
        let vector_storage = self.vector_storage.read();
        let ids = vector_storage.ids();
        let mut graph_cache = GraphMatchEvalCache::default();
        let mut groups: HashMap<GroupKey, Aggregator> = HashMap::new();

        for id in ids {
            let payload = payload_storage.retrieve(id).ok().flatten();
            let passes = if use_runtime {
                let mut rt_ctx = super::RuntimeWhereCtx {
                    vector_storage: &*vector_storage,
                    stmt,
                    params,
                    needs_vector_eval,
                    graph_cache: &mut graph_cache,
                };
                self.runtime_where_passes(id, payload.as_ref(), &mut rt_ctx)?
            } else if let Some(ref f) = filter {
                Self::payload_passes_filter(f, payload.as_ref())
            } else {
                true
            };
            if !passes {
                continue;
            }
            Self::insert_into_group(
                &mut groups,
                payload.as_ref(),
                group_by_columns,
                &columns_vec,
                has_count_star,
                max_groups,
            )?;
        }

        Ok(groups)
    }

    /// Checks whether runtime WHERE evaluation is needed for the given clause.
    fn needs_runtime_where_eval(where_clause: Option<&crate::velesql::Condition>) -> bool {
        where_clause.is_some_and(|cond| {
            Self::condition_contains_graph_match(cond) || Self::condition_requires_vector_eval(cond)
        })
    }

    /// Builds a static `Filter` from the WHERE clause when runtime eval is not needed.
    pub(super) fn build_static_filter(
        where_clause: Option<&crate::velesql::Condition>,
        use_runtime: bool,
        params: &HashMap<String, serde_json::Value>,
    ) -> Option<crate::filter::Filter> {
        if use_runtime {
            return None;
        }
        where_clause.map(|cond| {
            let resolved = Self::resolve_condition_params(cond, params);
            crate::filter::Filter::new(crate::filter::Condition::from(resolved))
        })
    }

    /// Extracts the list of columns to aggregate and whether COUNT(*) is present.
    pub(super) fn prepare_agg_columns(aggregations: &[AggregateFunction]) -> (Vec<String>, bool) {
        let mut seen = std::collections::HashSet::new();
        let columns: Vec<String> = aggregations
            .iter()
            .filter_map(|agg| match &agg.argument {
                AggregateArg::Column(col) => {
                    if seen.insert(col.clone()) {
                        Some(col.clone())
                    } else {
                        None
                    }
                }
                AggregateArg::Wildcard => None,
            })
            .collect();
        let has_count_star = aggregations
            .iter()
            .any(|agg| matches!(agg.argument, AggregateArg::Wildcard));
        (columns, has_count_star)
    }

    /// Inserts a record into the appropriate group, enforcing the max-groups limit.
    fn insert_into_group(
        groups: &mut HashMap<GroupKey, Aggregator>,
        payload: Option<&serde_json::Value>,
        group_by_columns: &[String],
        columns_to_aggregate: &[String],
        has_count_star: bool,
        max_groups: usize,
    ) -> Result<()> {
        let group_key = Self::extract_group_key_fast(payload, group_by_columns);
        if !groups.contains_key(&group_key) && groups.len() >= max_groups {
            return Err(crate::error::Error::Config(format!(
                "Too many groups (limit: {max_groups})"
            )));
        }

        let aggregator = groups.entry(group_key).or_default();
        Self::accumulate_record(aggregator, payload, columns_to_aggregate, has_count_star);
        Ok(())
    }

    /// Builds the result array from grouped aggregators, applying HAVING and ORDER BY.
    fn build_grouped_results(
        groups: HashMap<GroupKey, Aggregator>,
        aggregations: &[AggregateFunction],
        group_by_columns: &[String],
        having: Option<&HavingClause>,
        order_by: Option<&[crate::velesql::SelectOrderBy]>,
    ) -> Vec<serde_json::Value> {
        let mut results = Vec::new();

        for (group_key, aggregator) in groups {
            let agg_result = aggregator.finalize();
            if let Some(having_clause) = having {
                if !Self::evaluate_having(having_clause, &agg_result) {
                    continue;
                }
            }

            let mut row = serde_json::Map::new();
            for (i, col_name) in group_by_columns.iter().enumerate() {
                if let Some(val) = group_key.values.get(i) {
                    row.insert(col_name.clone(), val.clone());
                }
            }
            for agg in aggregations {
                let key = Self::aggregation_result_key(agg);
                let value = Self::aggregation_result_value(agg, &agg_result);
                row.insert(key, value);
            }
            results.push(serde_json::Value::Object(row));
        }

        if let Some(order_by) = order_by {
            Self::sort_aggregation_results(&mut results, order_by);
        }

        results
    }

    /// Compute the result key for an aggregation function.
    pub(crate) fn aggregation_result_key(agg: &AggregateFunction) -> String {
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
    pub(crate) fn aggregation_result_value(
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
