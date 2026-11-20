//! Aggregation execution for the WASM VelesQL executor (S4-13).
//!
//! Groups a scanned row set with core's engine-shared
//! [`Grouper`] — the same group set, group
//! rows, aggregates and HAVING verdicts as core — then orders the groups.
//! When GROUP BY is absent, all matching rows collapse into a single group
//! (implicit global group).
//!
//! A DISTINCT without aggregates is not handled here: it runs in the plain
//! SELECT pipeline through core's `apply_distinct`, which honours ORDER BY and
//! the default LIMIT (`velesql_select::finalize_plain`).
//!
//! # Layer contract
//!
//! The caller hands us the full scanned row set (after WHERE), then this
//! module groups it. Vector / similarity / fusion concerns live outside.

use velesdb_core::velesql::{
    GroupInput, GroupOutput, Grouper, HavingClause, HavingCondition, SelectColumns, SelectStatement,
};

use crate::exec_error::ExecError;
use crate::velesql_result::QueryResultRow;
use crate::velesql_value::Params;

/// A scanned row as seen by the aggregator: `(id, score, payload_object)`.
///
/// `payload` is `None` for rows that have no payload (rare — mostly in tests
/// or for synthetic vector-only rows). The aggregator handles it as "empty".
pub(crate) type ScannedRow<'a> = (u64, f32, Option<&'a serde_json::Value>);

/// Returns true if the SELECT uses any aggregation / GROUP BY / HAVING
/// feature that needs post-processing beyond the raw scan.
pub(crate) fn needs_aggregation_pipeline(stmt: &SelectStatement) -> bool {
    stmt.group_by.is_some() || stmt.having.is_some() || has_aggregate_columns(&stmt.columns)
}

/// Returns true if the SELECT column list contains any aggregate function.
pub(crate) fn has_aggregate_columns(cols: &SelectColumns) -> bool {
    match cols {
        SelectColumns::Aggregations(a) => !a.is_empty(),
        SelectColumns::Mixed { aggregations, .. } => !aggregations.is_empty(),
        _ => false,
    }
}

/// Groups `rows`, applies HAVING, then orders the groups: a vector GROUP BY
/// as core orders its search results (`velesql_vector_group`), any other
/// GROUP BY by the row fields its ORDER BY names. `higher_is_better` is the
/// collection metric's direction.
///
/// # Errors
///
/// Returns an error for an unbound HAVING parameter, a HAVING threshold that
/// is not a number (`V018`), more groups than `WITH (max_groups = N)` allows,
/// or an ORDER BY the grouped rows cannot be sorted by.
pub(crate) fn apply(
    stmt: &SelectStatement,
    rows: &[ScannedRow<'_>],
    params: &Params,
    higher_is_better: bool,
) -> Result<Vec<QueryResultRow>, ExecError> {
    let groups = group_rows(stmt, rows, params, higher_is_better)?;
    if stmt.is_vector_group_by() {
        return crate::velesql_vector_group::order_groups(stmt, groups, higher_is_better);
    }
    let mut json_rows: Vec<serde_json::Value> = groups
        .into_iter()
        .map(|group: GroupOutput| group.row)
        .collect();
    // Without GROUP BY the aggregates collapse into one row: there is nothing
    // to order, and core ignores that ORDER BY too. Without ORDER BY the
    // grouper's order (GROUP BY key ascending) stands.
    if stmt.group_by.is_some() && stmt.order_by.is_some() {
        sort_aggregate_rows(&mut json_rows, stmt)?;
    }
    json_rows
        .into_iter()
        .map(QueryResultRow::synthetic)
        .collect()
}

/// The groups of `rows`, HAVING (its parameters bound from `params`)
/// applied, in the grouper's order.
fn group_rows(
    stmt: &SelectStatement,
    rows: &[ScannedRow<'_>],
    params: &Params,
    higher_is_better: bool,
) -> Result<Vec<GroupOutput>, ExecError> {
    let having = stmt
        .having
        .as_ref()
        .map(|having| bind_having(having, params))
        .transpose()?;
    let mut grouper = Grouper::new(stmt, having.as_ref(), higher_is_better)?;
    // Id order, as core feeds its grouper: a float SUM or AVG then adds its
    // values in the same order in both engines.
    let mut ordered: Vec<&ScannedRow<'_>> = rows.iter().collect();
    ordered.sort_unstable_by_key(|row| row.0);
    for &&(id, score, payload) in &ordered {
        grouper.push(GroupInput::new(id, score, payload))?;
    }
    Ok(grouper.finish())
}

/// `having` with each threshold parameter bound from `params`.
fn bind_having(having: &HavingClause, params: &Params) -> Result<HavingClause, ExecError> {
    let conditions = having
        .conditions
        .iter()
        .map(|condition| {
            Ok(HavingCondition {
                aggregate: condition.aggregate.clone(),
                operator: condition.operator,
                // Core's binder, as core's HAVING binds it: the [`Grouper`]
                // then refuses (`V018`) a threshold that is not a number, in
                // both engines.
                value: velesdb_core::velesql::bind_value(&condition.value, params)?,
            })
        })
        .collect::<Result<Vec<_>, ExecError>>()?;
    Ok(HavingClause {
        conditions,
        operators: having.operators.clone(),
    })
}

/// Sorts grouped aggregate rows by the SELECT's ORDER BY, resolved to row
/// fields by core's `SelectStatement::grouped_sort_keys` — the resolution
/// core's own aggregate executor sorts through, GROUP BY columns last. The
/// sort is stable: groups tied on every key (two array keys) keep the
/// grouper's order, by best row id, as in core.
///
/// # Errors
///
/// Returns core's `V016` message for an ORDER BY item that names no field of
/// the grouped rows, rather than leaving them in scan order.
fn sort_aggregate_rows(
    rows: &mut [serde_json::Value],
    stmt: &SelectStatement,
) -> Result<(), ExecError> {
    let keys = stmt
        .grouped_sort_keys()
        .map_err(velesdb_core::Error::from)?;
    if !keys.is_empty() {
        rows.sort_by(|a, b| compare_aggregate_rows(a, b, &keys));
    }
    Ok(())
}

/// Total order over two aggregate rows for the resolved `(field, descending)`
/// keys, in core's ORDER BY value order (`velesql::compare_json_values`), the
/// one every ORDER BY path uses in both engines.
fn compare_aggregate_rows(
    a: &serde_json::Value,
    b: &serde_json::Value,
    keys: &[(String, bool)],
) -> std::cmp::Ordering {
    for (field, descending) in keys {
        let ord = velesdb_core::velesql::compare_json_values(a.get(field), b.get(field));
        let ord = if *descending { ord.reverse() } else { ord };
        if ord != std::cmp::Ordering::Equal {
            return ord;
        }
    }
    std::cmp::Ordering::Equal
}

#[cfg(test)]
#[path = "velesql_aggregate_unit_tests.rs"]
mod unit_tests;
