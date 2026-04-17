//! Aggregation execution for the WASM VelesQL executor (S4-13).
//!
//! Implements COUNT / SUM / AVG / MIN / MAX over a scanned row set, with
//! optional GROUP BY and HAVING clauses. When GROUP BY is absent, all
//! matching rows collapse into a single group (implicit global group).
//!
//! DISTINCT (`SELECT DISTINCT`) is also handled here since it is a pure-row
//! post-processing step and shares the "scan-then-collapse" pipeline.
//!
//! # Layer contract
//!
//! The caller hands us the full scanned row set (after WHERE), then this
//! module decides whether to return the raw rows, group them, or apply
//! DISTINCT. Vector / similarity / fusion concerns live outside.

use std::collections::BTreeMap;

use velesdb_core::velesql::{
    AggregateArg, AggregateFunction, AggregateType, CompareOp, DistinctMode, HavingClause,
    HavingCondition, LogicalOp, SelectColumns, SelectStatement, Value,
};

use crate::velesql_result::QueryResultRow;
use crate::velesql_value::{json_values_cmp, json_values_equal, resolve_value, Params};

/// A scanned row as seen by the aggregator: `(id, score, payload_object)`.
///
/// `payload` is `None` for rows that have no payload (rare — mostly in tests
/// or for synthetic vector-only rows). The aggregator handles it as "empty".
pub(crate) type ScannedRow<'a> = (u64, f32, Option<&'a serde_json::Value>);

/// Returns true if the SELECT uses any aggregation / GROUP BY / DISTINCT
/// feature that needs post-processing beyond the raw scan.
pub(crate) fn needs_aggregation_pipeline(stmt: &SelectStatement) -> bool {
    matches!(stmt.distinct, DistinctMode::All)
        || stmt.group_by.is_some()
        || stmt.having.is_some()
        || has_aggregate_columns(&stmt.columns)
}

/// Returns true if the SELECT column list contains any aggregate function.
pub(crate) fn has_aggregate_columns(cols: &SelectColumns) -> bool {
    match cols {
        SelectColumns::Aggregations(a) => !a.is_empty(),
        SelectColumns::Mixed { aggregations, .. } => !aggregations.is_empty(),
        _ => false,
    }
}

/// Applies the aggregation / grouping / DISTINCT pipeline to a scanned row
/// set, returning the final row list to surface through [`QueryResult`].
///
/// [`QueryResult`]: crate::velesql_result::QueryResult
pub(crate) fn apply(
    stmt: &SelectStatement,
    rows: &[ScannedRow<'_>],
    params: &Params,
) -> Result<Vec<QueryResultRow>, String> {
    if matches!(stmt.distinct, DistinctMode::All) && !has_aggregate_columns(&stmt.columns) {
        return dedup_rows(&stmt.columns, rows);
    }
    aggregate(stmt, rows, params)
}

// --- DISTINCT -------------------------------------------------------------

/// Deduplicates rows by the canonical JSON form of their selected columns.
///
/// DISTINCT without aggregations: project, then deduplicate preserving the
/// first-seen order. Ties are broken by insertion order to keep results
/// deterministic for JS callers.
fn dedup_rows(
    cols: &SelectColumns,
    rows: &[ScannedRow<'_>],
) -> Result<Vec<QueryResultRow>, String> {
    let mut seen: Vec<String> = Vec::with_capacity(rows.len());
    let mut out: Vec<QueryResultRow> = Vec::with_capacity(rows.len());
    for &(id, score, payload) in rows {
        let proj = project_for_distinct(cols, id, score, payload);
        let key = serde_json::to_string(&proj)
            .map_err(|e| format!("DISTINCT canonicalization failed: {e}"))?;
        if seen.iter().any(|k| k == &key) {
            continue;
        }
        seen.push(key);
        out.push(QueryResultRow::synthetic(proj)?);
    }
    Ok(out)
}

/// Builds the JSON object used both to display a DISTINCT row and to
/// canonicalize it for deduplication. The output is a proper JSON object so
/// stable key ordering follows serde's default (lexicographic) serialization.
fn project_for_distinct(
    cols: &SelectColumns,
    id: u64,
    score: f32,
    payload: Option<&serde_json::Value>,
) -> serde_json::Value {
    match cols {
        SelectColumns::Columns(cs) => {
            let mut map = serde_json::Map::new();
            for col in cs {
                map.insert(col.name.clone(), extract_column(&col.name, id, payload));
            }
            serde_json::Value::Object(map)
        }
        _ => {
            // DISTINCT * / DISTINCT on mixed — fall back to "full payload" key.
            let mut map = serde_json::Map::new();
            map.insert("id".to_string(), serde_json::json!(id));
            map.insert("score".to_string(), serde_json::json!(score));
            if let Some(serde_json::Value::Object(obj)) = payload {
                for (k, v) in obj {
                    if k != "id" && k != "score" {
                        map.insert(k.clone(), v.clone());
                    }
                }
            }
            serde_json::Value::Object(map)
        }
    }
}

// --- Aggregation ----------------------------------------------------------

/// Full aggregation pipeline: group rows, compute aggregates, apply HAVING,
/// serialize each group to a synthetic row.
fn aggregate(
    stmt: &SelectStatement,
    rows: &[ScannedRow<'_>],
    params: &Params,
) -> Result<Vec<QueryResultRow>, String> {
    let group_cols = stmt
        .group_by
        .as_ref()
        .map(|g| g.columns.clone())
        .unwrap_or_default();
    let groups = materialize_groups(&group_cols, rows, &stmt.columns);
    let aggregates = extract_aggregates(&stmt.columns);
    let plain_cols = extract_plain_columns(&stmt.columns);
    let mut out: Vec<QueryResultRow> = Vec::with_capacity(groups.len());
    for (key, group_rows) in groups {
        if let Some(row) = finalize_group(
            stmt,
            &group_cols,
            &key,
            &plain_cols,
            &aggregates,
            &group_rows,
            params,
        )? {
            out.push(row);
        }
    }
    Ok(out)
}

/// Returns the final row set after partitioning, adding the implicit global
/// group row when GROUP BY is absent and the SELECT contains aggregates.
fn materialize_groups<'a>(
    group_cols: &[String],
    rows: &[ScannedRow<'a>],
    columns: &SelectColumns,
) -> Vec<(Vec<serde_json::Value>, Vec<ScannedRow<'a>>)> {
    let mut groups = partition_into_groups(group_cols, rows);
    if groups.is_empty() && group_cols.is_empty() && has_aggregate_columns(columns) {
        groups.push((Vec::new(), Vec::new()));
    }
    groups
}

/// Builds the JSON row for a single group, applying HAVING last.
fn finalize_group(
    stmt: &SelectStatement,
    group_cols: &[String],
    key: &[serde_json::Value],
    plain_cols: &[String],
    aggregates: &[AggregateFunction],
    group_rows: &[ScannedRow<'_>],
    params: &Params,
) -> Result<Option<QueryResultRow>, String> {
    let mut payload = serde_json::Map::new();
    write_group_keys(&mut payload, group_cols, key);
    write_plain_columns(&mut payload, plain_cols, group_rows);
    write_aggregates(&mut payload, aggregates, group_rows)?;
    if !passes_having(aggregates, stmt.having.as_ref(), group_rows, params)? {
        return Ok(None);
    }
    Ok(Some(QueryResultRow::synthetic(serde_json::Value::Object(
        payload,
    ))?))
}

/// Partitions the row set into groups keyed by the GROUP BY columns.
///
/// When `group_cols` is empty, every row ends up in the single implicit
/// group keyed by `[]`. Preserves first-seen ordering of groups for
/// deterministic output.
fn partition_into_groups<'a>(
    group_cols: &[String],
    rows: &[ScannedRow<'a>],
) -> Vec<(Vec<serde_json::Value>, Vec<ScannedRow<'a>>)> {
    let mut ordered_keys: Vec<Vec<serde_json::Value>> = Vec::new();
    let mut buckets: BTreeMap<String, Vec<ScannedRow<'a>>> = BTreeMap::new();
    let mut dedup_keys: BTreeMap<String, Vec<serde_json::Value>> = BTreeMap::new();
    for &(id, score, payload) in rows {
        let key = group_key(group_cols, id, payload);
        let key_str = serde_json::to_string(&key).unwrap_or_default();
        if !dedup_keys.contains_key(&key_str) {
            ordered_keys.push(key.clone());
            dedup_keys.insert(key_str.clone(), key);
        }
        buckets
            .entry(key_str)
            .or_default()
            .push((id, score, payload));
    }
    ordered_keys
        .into_iter()
        .map(|k| {
            let key_str = serde_json::to_string(&k).unwrap_or_default();
            let bucket = buckets.remove(&key_str).unwrap_or_default();
            (k, bucket)
        })
        .collect()
}

/// Returns the vector of column values that forms the grouping key.
fn group_key(
    group_cols: &[String],
    id: u64,
    payload: Option<&serde_json::Value>,
) -> Vec<serde_json::Value> {
    group_cols
        .iter()
        .map(|c| extract_column(c, id, payload))
        .collect()
}

/// Reads a column from either the `id` pseudo-field or the payload JSON.
fn extract_column(column: &str, id: u64, payload: Option<&serde_json::Value>) -> serde_json::Value {
    if column == "id" {
        return serde_json::json!(id);
    }
    payload
        .and_then(|p| crate::filter::get_nested_field(p, column).cloned())
        .unwrap_or(serde_json::Value::Null)
}

// --- Selection-list utilities --------------------------------------------

fn extract_aggregates(cols: &SelectColumns) -> Vec<AggregateFunction> {
    match cols {
        SelectColumns::Aggregations(a) => a.clone(),
        SelectColumns::Mixed { aggregations, .. } => aggregations.clone(),
        _ => Vec::new(),
    }
}

fn extract_plain_columns(cols: &SelectColumns) -> Vec<String> {
    match cols {
        SelectColumns::Columns(c) => c.iter().map(|col| col.name.clone()).collect(),
        SelectColumns::Mixed { columns, .. } => {
            columns.iter().map(|col| col.name.clone()).collect()
        }
        _ => Vec::new(),
    }
}

fn write_group_keys(
    out: &mut serde_json::Map<String, serde_json::Value>,
    group_cols: &[String],
    key: &[serde_json::Value],
) {
    for (col, val) in group_cols.iter().zip(key.iter()) {
        out.insert(col.clone(), val.clone());
    }
}

fn write_plain_columns(
    out: &mut serde_json::Map<String, serde_json::Value>,
    plain: &[String],
    rows: &[ScannedRow<'_>],
) {
    if plain.is_empty() || rows.is_empty() {
        return;
    }
    // Non-aggregated columns use the first row's value. This matches SQL's
    // "any-value" behaviour when a column is not in GROUP BY (undefined by
    // spec but stable for a deterministic single-row sample).
    let &(id, _, payload) = &rows[0];
    for col in plain {
        if !out.contains_key(col) {
            out.insert(col.clone(), extract_column(col, id, payload));
        }
    }
}

fn write_aggregates(
    out: &mut serde_json::Map<String, serde_json::Value>,
    aggregates: &[AggregateFunction],
    rows: &[ScannedRow<'_>],
) -> Result<(), String> {
    for agg in aggregates {
        let value = compute_aggregate(agg, rows)?;
        let name = agg
            .alias
            .clone()
            .unwrap_or_else(|| aggregate_default_name(agg));
        out.insert(name, value);
    }
    Ok(())
}

fn aggregate_default_name(agg: &AggregateFunction) -> String {
    let fn_name = match agg.function_type {
        AggregateType::Count => "count",
        AggregateType::Sum => "sum",
        AggregateType::Avg => "avg",
        AggregateType::Min => "min",
        AggregateType::Max => "max",
        AggregateType::First => "first",
        _ => "agg", // forward-compat for #[non_exhaustive]
    };
    let arg_name = match &agg.argument {
        AggregateArg::Wildcard => "*".to_string(),
        AggregateArg::Column(c) => c.clone(),
        AggregateArg::Score => "score".to_string(),
        _ => "value".to_string(),
    };
    format!("{fn_name}({arg_name})")
}

pub(crate) fn compute_aggregate(
    agg: &AggregateFunction,
    rows: &[ScannedRow<'_>],
) -> Result<serde_json::Value, String> {
    match agg.function_type {
        AggregateType::Count => Ok(serde_json::json!(count(&agg.argument, rows))),
        AggregateType::Sum => Ok(serde_json::json!(sum(&agg.argument, rows))),
        AggregateType::Avg => Ok(avg_to_json(&agg.argument, rows)),
        AggregateType::Min => Ok(min_max(&agg.argument, rows, true)),
        AggregateType::Max => Ok(min_max(&agg.argument, rows, false)),
        AggregateType::First => Ok(first_value(&agg.argument, rows)),
        _ => Err(format!(
            "Unsupported aggregate function in WASM: {:?}",
            agg.function_type
        )),
    }
}

fn count(arg: &AggregateArg, rows: &[ScannedRow<'_>]) -> u64 {
    match arg {
        AggregateArg::Wildcard => rows.len() as u64,
        AggregateArg::Column(c) => rows
            .iter()
            .filter(|(id, _, p)| {
                let v = extract_column(c, *id, *p);
                !v.is_null()
            })
            .count() as u64,
        AggregateArg::Score => rows.iter().filter(|(_, s, _)| !s.is_nan()).count() as u64,
        // Forward-compat: unknown argument counts as 0 rather than panicking.
        _ => 0,
    }
}

fn sum(arg: &AggregateArg, rows: &[ScannedRow<'_>]) -> f64 {
    numeric_values(arg, rows).into_iter().sum()
}

fn avg_to_json(arg: &AggregateArg, rows: &[ScannedRow<'_>]) -> serde_json::Value {
    let values = numeric_values(arg, rows);
    if values.is_empty() {
        return serde_json::Value::Null;
    }
    #[allow(clippy::cast_precision_loss)]
    let avg = values.iter().sum::<f64>() / values.len() as f64;
    serde_json::json!(avg)
}

fn min_max(arg: &AggregateArg, rows: &[ScannedRow<'_>], pick_min: bool) -> serde_json::Value {
    let values: Vec<serde_json::Value> = collect_values(arg, rows);
    if values.is_empty() {
        return serde_json::Value::Null;
    }
    let mut best = values[0].clone();
    for v in values.iter().skip(1) {
        let cmp = json_values_cmp(v, &best);
        match (cmp, pick_min) {
            (Some(std::cmp::Ordering::Less), true) | (Some(std::cmp::Ordering::Greater), false) => {
                best = v.clone();
            }
            _ => {}
        }
    }
    best
}

fn first_value(arg: &AggregateArg, rows: &[ScannedRow<'_>]) -> serde_json::Value {
    collect_values(arg, rows)
        .into_iter()
        .next()
        .unwrap_or(serde_json::Value::Null)
}

fn collect_values(arg: &AggregateArg, rows: &[ScannedRow<'_>]) -> Vec<serde_json::Value> {
    rows.iter()
        .filter_map(|(id, score, payload)| match arg {
            AggregateArg::Column(c) => {
                let v = extract_column(c, *id, *payload);
                (!v.is_null()).then_some(v)
            }
            AggregateArg::Score => Some(serde_json::json!(*score)),
            AggregateArg::Wildcard => Some(serde_json::json!(*id)),
            _ => None,
        })
        .collect()
}

fn numeric_values(arg: &AggregateArg, rows: &[ScannedRow<'_>]) -> Vec<f64> {
    collect_values(arg, rows)
        .into_iter()
        .filter_map(|v| v.as_f64())
        .collect()
}

// --- HAVING ---------------------------------------------------------------

fn passes_having(
    aggregates: &[AggregateFunction],
    having: Option<&HavingClause>,
    rows: &[ScannedRow<'_>],
    params: &Params,
) -> Result<bool, String> {
    let Some(having) = having else {
        return Ok(true);
    };
    if having.conditions.is_empty() {
        return Ok(true);
    }

    let mut accumulator: Option<bool> = None;
    for (i, cond) in having.conditions.iter().enumerate() {
        let outcome = evaluate_having_condition(cond, aggregates, rows, params)?;
        accumulator = Some(combine_having(accumulator, outcome, &having.operators, i));
    }
    Ok(accumulator.unwrap_or(true))
}

fn combine_having(acc: Option<bool>, outcome: bool, ops: &[LogicalOp], idx: usize) -> bool {
    let Some(prev) = acc else {
        return outcome;
    };
    let op = ops
        .get(idx.saturating_sub(1))
        .copied()
        .unwrap_or(LogicalOp::And);
    match op {
        LogicalOp::And => prev && outcome,
        LogicalOp::Or => prev || outcome,
        // Forward-compat: LogicalOp is #[non_exhaustive]; default to AND.
        _ => prev && outcome,
    }
}

fn evaluate_having_condition(
    cond: &HavingCondition,
    _aggregates: &[AggregateFunction],
    rows: &[ScannedRow<'_>],
    params: &Params,
) -> Result<bool, String> {
    let left = compute_aggregate(&cond.aggregate, rows)?;
    let right = resolve_value_from_ast(&cond.value, params)?;
    let op = cond.operator;
    Ok(apply_compare(op, &left, &right))
}

fn resolve_value_from_ast(v: &Value, params: &Params) -> Result<serde_json::Value, String> {
    resolve_value(v, params)
}

fn apply_compare(op: CompareOp, left: &serde_json::Value, right: &serde_json::Value) -> bool {
    match op {
        CompareOp::Eq => json_values_equal(left, right),
        CompareOp::NotEq => !json_values_equal(left, right),
        CompareOp::Gt => json_values_cmp(left, right) == Some(std::cmp::Ordering::Greater),
        CompareOp::Gte => matches!(
            json_values_cmp(left, right),
            Some(std::cmp::Ordering::Greater | std::cmp::Ordering::Equal)
        ),
        CompareOp::Lt => json_values_cmp(left, right) == Some(std::cmp::Ordering::Less),
        CompareOp::Lte => matches!(
            json_values_cmp(left, right),
            Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal)
        ),
        _ => false,
    }
}

#[cfg(test)]
#[path = "velesql_aggregate_unit_tests.rs"]
mod unit_tests;
