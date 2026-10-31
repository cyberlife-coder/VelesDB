//! HAVING parameter binding and aggregation result sorting.
//!
//! Extracted from `grouped.rs` for single-responsibility:
//! - Sorting grouped results by ORDER BY clause
//! - JSON value comparison utilities for aggregation ordering
//! - Parameter resolution for condition placeholders

// Reason: Numeric casts in aggregation are intentional:
// - All casts are for computing aggregate statistics (sum, avg, count)
// - i64->usize for group limits: limits bounded by MAX_GROUPS (1M)
// - Values bounded by result set size and field cardinality
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_sign_loss)]

use crate::collection::types::Collection;
use crate::velesql::{HavingClause, Value};
use std::collections::HashMap;

/// Default maximum number of groups allowed when the query does not specify one
/// (memory protection).
pub(super) const DEFAULT_MAX_GROUPS: usize = 10_000;

/// Server-side hard ceiling on the number of GROUP BY groups (#903).
///
/// This is a SERVER-controlled constant. A query may use `WITH (max_groups=N)`
/// to *lower* its group budget, but `N` is always clamped down to this ceiling
/// so an untrusted query cannot inflate the per-query memory bound. Bounding
/// distinct groups is the DoS guard for GROUP BY aggregation.
pub(super) const SERVER_MAX_GROUPS_CEILING: usize = 1_000_000;

impl Collection {
    /// Sorts grouped rows by `(field, descending)` keys, in order — the row
    /// fields [`crate::velesql::SelectStatement::grouped_sort_keys`] resolved
    /// the ORDER BY to.
    pub(crate) fn sort_aggregation_results(
        results: &mut [serde_json::Value],
        sort_columns: &[(String, bool)],
    ) {
        if sort_columns.is_empty() {
            return;
        }
        results.sort_unstable_by(|a, b| {
            for (column, descending) in sort_columns {
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

    /// Extract max_groups limit from WITH clause (EPIC-040 US-004).
    /// Supports both `max_groups` and `group_limit` option names.
    /// Returns `DEFAULT_MAX_GROUPS` if not specified.
    ///
    /// #903 (DoS hardening): the value supplied by the (untrusted) query is
    /// **clamped down** to [`SERVER_MAX_GROUPS_CEILING`]. The query can lower
    /// the limit but can never raise the server-side memory ceiling above it.
    pub(super) fn extract_max_groups_limit(
        with_clause: Option<&crate::velesql::WithClause>,
    ) -> usize {
        let Some(with) = with_clause else {
            return DEFAULT_MAX_GROUPS;
        };

        for opt in &with.options {
            if opt.key == "max_groups" || opt.key == "group_limit" {
                // Try to parse value as integer
                if let crate::velesql::WithValue::Integer(n) = &opt.value {
                    // Ensure positive, then clamp to the server-side ceiling.
                    // The query CANNOT raise the limit above SERVER_MAX_GROUPS_CEILING.
                    let limit = (*n).max(1) as usize;
                    return limit.min(SERVER_MAX_GROUPS_CEILING);
                }
            }
        }

        DEFAULT_MAX_GROUPS
    }

    /// Resolves parameter placeholders in HAVING threshold values.
    ///
    /// HAVING thresholds live in [`HavingCondition.value`], outside the WHERE
    /// condition tree, so the WHERE resolver never visits them. Without this,
    /// `HAVING COUNT(*) > $n` compares every group against an unresolved
    /// placeholder and silently filters out all groups — even when `$n` is
    /// bound.
    ///
    /// [`HavingCondition.value`]: crate::velesql::HavingCondition
    ///
    /// # Errors
    ///
    /// Returns an error when a referenced parameter is missing from `params`
    /// or has an unsupported type (array/object).
    pub(crate) fn resolve_having_params(
        having: &HavingClause,
        params: &HashMap<String, serde_json::Value>,
    ) -> crate::error::Result<HavingClause> {
        let conditions = having
            .conditions
            .iter()
            .map(|cond| {
                Ok(crate::velesql::HavingCondition {
                    aggregate: cond.aggregate.clone(),
                    operator: cond.operator,
                    value: Self::resolve_where_param(&cond.value, params)?,
                })
            })
            .collect::<crate::error::Result<Vec<_>>>()?;
        Ok(HavingClause {
            conditions,
            operators: having.operators.clone(),
        })
    }

    /// BUG-5 FIX: Resolve parameter placeholders in a condition.
    /// Replaces `Value::Parameter("name")` with the actual value from params `HashMap`.
    ///
    /// Delegates leaf resolution to [`Self::resolve_where_param`] (the single
    /// strict resolver) so a missing or unsupported parameter is an error,
    /// never a silent `NULL`.
    ///
    /// # Errors
    ///
    /// Returns an error when a referenced parameter is missing from `params`
    /// or has an unsupported type (array/object).
    pub(crate) fn resolve_condition_params(
        cond: &crate::velesql::Condition,
        params: &HashMap<String, serde_json::Value>,
    ) -> crate::error::Result<crate::velesql::Condition> {
        use crate::velesql::Condition;

        match cond {
            Condition::And(left, right) => Ok(Condition::And(
                Self::resolve_boxed_condition(left, params)?,
                Self::resolve_boxed_condition(right, params)?,
            )),
            Condition::Or(left, right) => Ok(Condition::Or(
                Self::resolve_boxed_condition(left, params)?,
                Self::resolve_boxed_condition(right, params)?,
            )),
            Condition::Not(inner) => Ok(Condition::Not(Self::resolve_boxed_condition(
                inner, params,
            )?)),
            Condition::Group(inner) => Ok(Condition::Group(Self::resolve_boxed_condition(
                inner, params,
            )?)),
            other => Self::resolve_leaf_condition_params(other, params),
        }
    }

    /// Boxes the recursive resolution of a nested condition.
    fn resolve_boxed_condition(
        cond: &crate::velesql::Condition,
        params: &HashMap<String, serde_json::Value>,
    ) -> crate::error::Result<Box<crate::velesql::Condition>> {
        Ok(Box::new(Self::resolve_condition_params(cond, params)?))
    }

    /// Resolves parameters in leaf conditions (Comparison, IN, BETWEEN,
    /// CONTAINS / CONTAINS ANY / CONTAINS ALL).
    ///
    /// The remaining leaf variants are cloned unchanged: they carry no scalar
    /// `Value` operands (geo thresholds are `f64` literals, LIKE/MATCH
    /// patterns are strings, vector parameters are resolved by the vector
    /// pipeline), except `GraphMatch`, whose pattern properties are evaluated
    /// by the MATCH engine rather than by this resolver.
    fn resolve_leaf_condition_params(
        cond: &crate::velesql::Condition,
        params: &HashMap<String, serde_json::Value>,
    ) -> crate::error::Result<crate::velesql::Condition> {
        use crate::velesql::Condition;

        Ok(match cond {
            Condition::Comparison(cmp) => Condition::Comparison(crate::velesql::Comparison {
                column: cmp.column.clone(),
                operator: cmp.operator,
                value: Self::resolve_where_param(&cmp.value, params)?,
            }),
            Condition::In(in_cond) => Condition::In(crate::velesql::InCondition {
                column: in_cond.column.clone(),
                values: Self::resolve_value_list(&in_cond.values, params)?,
                negated: in_cond.negated,
            }),
            Condition::Between(btw) => Condition::Between(crate::velesql::BetweenCondition {
                column: btw.column.clone(),
                low: Self::resolve_where_param(&btw.low, params)?,
                high: Self::resolve_where_param(&btw.high, params)?,
            }),
            Condition::Contains(contains) => {
                Condition::Contains(crate::velesql::ContainsCondition {
                    column: contains.column.clone(),
                    mode: contains.mode,
                    values: Self::resolve_value_list(&contains.values, params)?,
                })
            }
            // These conditions don't have Value parameters to resolve
            other => other.clone(),
        })
    }

    /// Resolves every value in a list via [`Self::resolve_where_param`].
    fn resolve_value_list(
        values: &[Value],
        params: &HashMap<String, serde_json::Value>,
    ) -> crate::error::Result<Vec<Value>> {
        values
            .iter()
            .map(|v| Self::resolve_where_param(v, params))
            .collect()
    }
}
