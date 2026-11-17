//! Grouped-row ordering and HAVING parameter binding.
//!
//! Extracted from `grouped.rs` for single-responsibility:
//! - Sorting grouped rows by the keys their ORDER BY resolved to
//! - Parameter resolution for HAVING and WHERE placeholders
//!
//! HAVING itself — its verdict and its group budget — is evaluated by the
//! engine-shared [`crate::velesql::Grouper`].

use crate::collection::types::Collection;
use crate::velesql::HavingClause;
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
                    value: crate::velesql::bind_value(&cond.value, params)?,
                })
            })
            .collect::<crate::error::Result<Vec<_>>>()?;
        Ok(HavingClause {
            conditions,
            operators: having.operators.clone(),
        })
    }
}
