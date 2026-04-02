//! HAVING clause evaluation and aggregation result sorting.
//!
//! Extracted from `grouped.rs` for single-responsibility:
//! - HAVING filter evaluation against aggregation results
//! - Sorting grouped results by ORDER BY clause
//! - JSON value comparison utilities for aggregation ordering
//! - Parameter resolution for condition placeholders

// SAFETY: Numeric casts in aggregation are intentional:
// - All casts are for computing aggregate statistics (sum, avg, count)
// - i64->usize for group limits: limits bounded by MAX_GROUPS (1M)
// - Values bounded by result set size and field cardinality
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_sign_loss)]

use crate::collection::types::Collection;
use crate::velesql::{
    AggregateArg, AggregateFunction, AggregateResult, AggregateType, CompareOp, HavingClause, Value,
};
use std::collections::HashMap;

impl Collection {
    /// BUG-3 FIX: Sort aggregation results by ORDER BY clause.
    pub(crate) fn sort_aggregation_results(
        results: &mut [serde_json::Value],
        order_by: &[crate::velesql::SelectOrderBy],
    ) {
        use crate::velesql::OrderByExpr;

        let sort_columns: Vec<(String, bool)> = order_by
            .iter()
            .filter_map(|clause| {
                let column = match &clause.expr {
                    OrderByExpr::Field(name) => name.clone(),
                    OrderByExpr::Aggregate(agg) => Self::aggregation_result_key(agg),
                    // Similarity/Arithmetic ordering not applicable to grouped aggregate rows.
                    OrderByExpr::Similarity(_)
                    | OrderByExpr::SimilarityBare
                    | OrderByExpr::Arithmetic(_) => return None,
                };
                Some((column, clause.descending))
            })
            .collect();

        results.sort_by(|a, b| {
            for (column, descending) in &sort_columns {
                let val_a = a.get(column);
                let val_b = b.get(column);

                let ordering =
                    crate::collection::search::query::ordering::compare_json_values(val_a, val_b);

                let ordering = if *descending {
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

    /// Extract group key from payload with pre-computed hash (optimized).
    /// Avoids JSON serialization overhead by using direct value hashing.
    pub(super) fn extract_group_key_fast(
        payload: Option<&serde_json::Value>,
        group_by_columns: &[String],
    ) -> super::GroupKey {
        let values: Vec<serde_json::Value> = group_by_columns
            .iter()
            .map(|col| {
                payload
                    .and_then(|p| Self::get_nested_value(p, col).cloned())
                    .unwrap_or(serde_json::Value::Null)
            })
            .collect();
        super::GroupKey::new(values)
    }

    /// Evaluate HAVING clause against aggregation result.
    /// Supports both AND and OR logical operators between conditions.
    pub(super) fn evaluate_having(having: &HavingClause, agg_result: &AggregateResult) -> bool {
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
            #[allow(clippy::cast_precision_loss)]
            // Reason: aggregate comparison converts to f64; precision loss is
            // acceptable for large u64 values in HAVING threshold context.
            Value::UnsignedInteger(u) => *u as f64,
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
    /// Returns `DEFAULT_MAX_GROUPS` if not specified.
    pub(super) fn extract_max_groups_limit(
        with_clause: Option<&crate::velesql::WithClause>,
    ) -> usize {
        /// Default maximum number of groups allowed (memory protection).
        const DEFAULT_MAX_GROUPS: usize = 10000;

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
    /// Replaces `Value::Parameter("name")` with the actual value from params `HashMap`.
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
                    negated: in_cond.negated,
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
                            } else if let Some(u) = n.as_u64() {
                                Value::UnsignedInteger(u)
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
