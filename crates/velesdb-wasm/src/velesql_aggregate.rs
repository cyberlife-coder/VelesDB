//! Aggregation execution for the WASM VelesQL executor (S4-13).
//!
//! Groups a scanned row set with core's engine-shared
//! [`Grouper`](velesdb_core::velesql::Grouper) — the same group set, group
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
    GroupInput, GroupOutput, Grouper, HavingClause, HavingCondition, SelectColumns,
    SelectStatement, Value,
};

use crate::velesql_result::QueryResultRow;
use crate::velesql_value::{resolve_value, Params};

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
) -> Result<Vec<QueryResultRow>, String> {
    let having = stmt
        .having
        .as_ref()
        .map(|having| bind_having(having, params))
        .transpose()?;
    let mut grouper =
        Grouper::new(stmt, having.as_ref(), higher_is_better).map_err(|e| e.to_string())?;
    // Id order, as core feeds its grouper: a float SUM or AVG then adds its
    // values in the same order in both engines.
    let mut ordered: Vec<&ScannedRow<'_>> = rows.iter().collect();
    ordered.sort_unstable_by_key(|row| row.0);
    for &&(id, score, payload) in &ordered {
        grouper
            .push(GroupInput::new(id, score, payload))
            .map_err(|e| e.to_string())?;
    }
    let groups = grouper.finish();
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

/// `having` with each threshold parameter bound from `params`.
fn bind_having(having: &HavingClause, params: &Params) -> Result<HavingClause, String> {
    let conditions = having
        .conditions
        .iter()
        .map(|condition| {
            Ok(HavingCondition {
                aggregate: condition.aggregate.clone(),
                operator: condition.operator,
                value: threshold(&condition.value, params)?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(HavingClause {
        conditions,
        operators: having.operators.clone(),
    })
}

/// A HAVING threshold with a parameter bound to the literal its JSON value
/// is, as core binds it: the [`Grouper`] then refuses (`V018`) one that is
/// not a number, in both engines. A literal threshold is kept: validation
/// already refused a non-numeric one.
fn threshold(value: &Value, params: &Params) -> Result<Value, String> {
    if !matches!(value, Value::Parameter(_)) {
        return Ok(value.clone());
    }
    Ok(match resolve_value(value, params)? {
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::Integer(i)
            } else if let Some(u) = n.as_u64() {
                Value::UnsignedInteger(u)
            } else {
                Value::Float(n.as_f64().unwrap_or(f64::NAN))
            }
        }
        serde_json::Value::String(s) => Value::String(s),
        serde_json::Value::Bool(b) => Value::Boolean(b),
        serde_json::Value::Null => Value::Null,
        other => return Err(format!("Unsupported parameter type for HAVING: {other}")),
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
) -> Result<(), String> {
    let keys = stmt.grouped_sort_keys().map_err(|e| e.to_string())?;
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
