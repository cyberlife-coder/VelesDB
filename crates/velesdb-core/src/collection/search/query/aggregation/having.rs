//! Grouped-row ordering and HAVING parameter binding.
//!
//! Extracted from `grouped.rs` for single-responsibility:
//! - Sorting grouped rows by the keys their ORDER BY resolved to
//! - Parameter resolution for HAVING and WHERE placeholders
//!
//! HAVING itself — its verdict and its group budget — is evaluated by the
//! engine-shared [`crate::velesql::Grouper`].

use crate::collection::types::Collection;
use crate::velesql::{HavingClause, Value};
use std::collections::HashMap;

impl Collection {
    /// Sorts grouped rows by `(field, descending)` keys, in order — the row
    /// fields [`crate::velesql::SelectStatement::grouped_sort_keys`] resolved
    /// the ORDER BY to, GROUP BY columns last. The sort is stable: groups
    /// tied on every key (two array keys) keep the grouper's order, by best
    /// row id.
    pub(crate) fn sort_aggregation_results(
        results: &mut [serde_json::Value],
        sort_columns: &[(String, bool)],
    ) {
        if sort_columns.is_empty() {
            return;
        }
        results.sort_by(|a, b| {
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
