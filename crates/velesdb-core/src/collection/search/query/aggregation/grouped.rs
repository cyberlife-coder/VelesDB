//! Grouped aggregation (GROUP BY) with HAVING filters and parameter resolution.
//!
//! Extracted from aggregation.rs for complexity reduction (Plan 04-04).

// SAFETY: Numeric casts in aggregation are intentional:
// - All casts are for computing aggregate statistics (sum, avg, count)
// - i64->usize for group limits: limits bounded by MAX_GROUPS (1M)
// - Values bounded by result set size and field cardinality
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_sign_loss)]

use crate::collection::types::Collection;
use crate::error::Result;
use crate::storage::{PayloadStorage, VectorStorage};
use crate::velesql::{
    AggregateArg, AggregateFunction, AggregateResult, AggregateType, Aggregator, CompareOp,
    HavingClause, Query, Value,
};
use std::collections::HashMap;

use super::GroupKey;

/// Default maximum number of groups allowed (memory protection).
/// Can be overridden via WITH(max_groups=N) or WITH(group_limit=N).
const DEFAULT_MAX_GROUPS: usize = 10000;

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

        // BUG-5 FIX: Build filter from WHERE clause with parameter resolution
        let filter = stmt.where_clause.as_ref().map(|cond| {
            let resolved = Self::resolve_condition_params(cond, params);
            crate::filter::Filter::new(crate::filter::Condition::from(resolved))
        });

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

        // BUG-3 FIX: Apply ORDER BY to grouped aggregation results
        if let Some(ref order_by) = stmt.order_by {
            Self::sort_aggregation_results(&mut results, order_by);
        }

        Ok(serde_json::Value::Array(results))
    }

    /// BUG-3 FIX: Sort aggregation results by ORDER BY clause.
    fn sort_aggregation_results(
        results: &mut [serde_json::Value],
        order_by: &[crate::velesql::SelectOrderBy],
    ) {
        use crate::velesql::OrderByExpr;

        results.sort_by(|a, b| {
            for ob in order_by {
                let ordering = match &ob.expr {
                    OrderByExpr::Field(field) => {
                        let val_a = a.get(field);
                        let val_b = b.get(field);
                        Self::compare_json_values(val_a, val_b)
                    }
                    OrderByExpr::Aggregate(agg) => {
                        // Get the key name for this aggregate
                        let key = if let Some(ref alias) = agg.alias {
                            alias.clone()
                        } else {
                            match &agg.argument {
                                crate::velesql::AggregateArg::Wildcard => "count".to_string(),
                                crate::velesql::AggregateArg::Column(col) => {
                                    let prefix = match agg.function_type {
                                        crate::velesql::AggregateType::Count => "count",
                                        crate::velesql::AggregateType::Sum => "sum",
                                        crate::velesql::AggregateType::Avg => "avg",
                                        crate::velesql::AggregateType::Min => "min",
                                        crate::velesql::AggregateType::Max => "max",
                                    };
                                    format!("{prefix}_{col}")
                                }
                            }
                        };
                        let val_a = a.get(&key);
                        let val_b = b.get(&key);
                        Self::compare_json_values(val_a, val_b)
                    }
                    OrderByExpr::Similarity(_) => std::cmp::Ordering::Equal, // Not applicable for aggregations
                };

                let ordering = if ob.descending {
                    ordering.reverse()
                } else {
                    ordering
                };

                if ordering != std::cmp::Ordering::Equal {
                    return ordering;
                }
            }
            std::cmp::Ordering::Equal
        });
    }

    /// Compare two JSON values for sorting.
    pub(crate) fn compare_json_values(
        a: Option<&serde_json::Value>,
        b: Option<&serde_json::Value>,
    ) -> std::cmp::Ordering {
        match (a, b) {
            (None, None) => std::cmp::Ordering::Equal,
            (None, Some(_)) => std::cmp::Ordering::Less,
            (Some(_), None) => std::cmp::Ordering::Greater,
            (Some(va), Some(vb)) => {
                // Numeric comparison
                if let (Some(na), Some(nb)) = (va.as_f64(), vb.as_f64()) {
                    return na.total_cmp(&nb);
                }
                // String comparison
                if let (Some(sa), Some(sb)) = (va.as_str(), vb.as_str()) {
                    return sa.cmp(sb);
                }
                // Fallback: compare as strings
                va.to_string().cmp(&vb.to_string())
            }
        }
    }

    /// Extract group key from payload with pre-computed hash (optimized).
    /// Avoids JSON serialization overhead by using direct value hashing.
    fn extract_group_key_fast(
        payload: Option<&serde_json::Value>,
        group_by_columns: &[String],
    ) -> GroupKey {
        let values: Vec<serde_json::Value> = group_by_columns
            .iter()
            .map(|col| {
                payload
                    .and_then(|p| Self::get_nested_value(p, col).cloned())
                    .unwrap_or(serde_json::Value::Null)
            })
            .collect();
        GroupKey::new(values)
    }

    /// Evaluate HAVING clause against aggregation result.
    /// Supports both AND and OR logical operators between conditions.
    fn evaluate_having(having: &HavingClause, agg_result: &AggregateResult) -> bool {
        if having.conditions.is_empty() {
            return true;
        }

        // Evaluate first condition
        let mut result = {
            let cond = &having.conditions[0];
            let agg_value = Self::get_aggregate_value(&cond.aggregate, agg_result);
            Self::compare_values(agg_value, cond.operator, &cond.value)
        };

        // Apply remaining conditions with their operators
        for (i, cond) in having.conditions.iter().enumerate().skip(1) {
            let cond_result = {
                let agg_value = Self::get_aggregate_value(&cond.aggregate, agg_result);
                Self::compare_values(agg_value, cond.operator, &cond.value)
            };

            // Get operator (default to AND if not specified - backward compatible)
            let op = having
                .operators
                .get(i - 1)
                .copied()
                .unwrap_or(crate::velesql::LogicalOp::And);

            match op {
                crate::velesql::LogicalOp::And => result = result && cond_result,
                crate::velesql::LogicalOp::Or => result = result || cond_result,
            }
        }

        result
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
        let Some(agg) = agg_value else {
            return false;
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

    /// Extract max_groups limit from WITH clause (EPIC-040 US-004).
    /// Supports both `max_groups` and `group_limit` option names.
    /// Returns DEFAULT_MAX_GROUPS if not specified.
    fn extract_max_groups_limit(with_clause: Option<&crate::velesql::WithClause>) -> usize {
        let Some(with) = with_clause else {
            return DEFAULT_MAX_GROUPS;
        };

        for opt in &with.options {
            if opt.key == "max_groups" || opt.key == "group_limit" {
                // Try to parse value as integer
                if let crate::velesql::WithValue::Integer(n) = &opt.value {
                    // Ensure positive and reasonable limit
                    let limit = (*n).max(1) as usize;
                    return limit.min(1_000_000); // Hard cap at 1M groups
                }
            }
        }

        DEFAULT_MAX_GROUPS
    }

    /// BUG-5 FIX: Resolve parameter placeholders in a condition.
    /// Replaces Value::Parameter("name") with the actual value from params HashMap.
    pub(crate) fn resolve_condition_params(
        cond: &crate::velesql::Condition,
        params: &HashMap<String, serde_json::Value>,
    ) -> crate::velesql::Condition {
        use crate::velesql::Condition;

        match cond {
            Condition::Comparison(cmp) => {
                let resolved_value = Self::resolve_value(&cmp.value, params);
                Condition::Comparison(crate::velesql::Comparison {
                    column: cmp.column.clone(),
                    operator: cmp.operator,
                    value: resolved_value,
                })
            }
            Condition::In(in_cond) => {
                let resolved_values: Vec<Value> = in_cond
                    .values
                    .iter()
                    .map(|v| Self::resolve_value(v, params))
                    .collect();
                Condition::In(crate::velesql::InCondition {
                    column: in_cond.column.clone(),
                    values: resolved_values,
                })
            }
            Condition::Between(btw) => {
                let resolved_low = Self::resolve_value(&btw.low, params);
                let resolved_high = Self::resolve_value(&btw.high, params);
                Condition::Between(crate::velesql::BetweenCondition {
                    column: btw.column.clone(),
                    low: resolved_low,
                    high: resolved_high,
                })
            }
            Condition::And(left, right) => Condition::And(
                Box::new(Self::resolve_condition_params(left, params)),
                Box::new(Self::resolve_condition_params(right, params)),
            ),
            Condition::Or(left, right) => Condition::Or(
                Box::new(Self::resolve_condition_params(left, params)),
                Box::new(Self::resolve_condition_params(right, params)),
            ),
            Condition::Not(inner) => {
                Condition::Not(Box::new(Self::resolve_condition_params(inner, params)))
            }
            Condition::Group(inner) => {
                Condition::Group(Box::new(Self::resolve_condition_params(inner, params)))
            }
            // These conditions don't have Value parameters to resolve
            other => other.clone(),
        }
    }

    /// Resolve a single Value, substituting Parameter with actual value from params.
    pub(crate) fn resolve_value(
        value: &Value,
        params: &HashMap<String, serde_json::Value>,
    ) -> Value {
        match value {
            Value::Parameter(name) => {
                if let Some(param_value) = params.get(name) {
                    // Convert serde_json::Value to VelesQL Value
                    match param_value {
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
                        // Null, arrays, and objects not supported as params
                        _ => Value::Null,
                    }
                } else {
                    // Parameter not found, keep as null
                    Value::Null
                }
            }
            other => other.clone(),
        }
    }
}
