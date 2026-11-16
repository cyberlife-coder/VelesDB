//! JOIN execution for the WASM VelesQL executor.
//!
//! INNER and LEFT JOIN by a nested loop — fine for browser-sized datasets —
//! on one equality (`ON a.col = b.col`, oriented as core orients it, or
//! `USING (col)`). A joined row is built as core builds it
//! ([`velesdb_core::velesql::scope_base`] / [`velesdb_core::velesql::merge_side`]):
//! the base point's id and score, each side's fields flat and under its alias.
//! The joined rows then go through the SELECT pipeline like any scan — WHERE
//! here, then DISTINCT, ORDER BY, projection and OFFSET / LIMIT in
//! `velesql_select` — so `o.amount`, `c.name` and a self-join's two `name`s
//! read as in core.

use velesdb_core::velesql::json_path::payload_path;
use velesdb_core::velesql::{
    merge_side, scope_base, side_alias, JoinClause, JoinType, SelectStatement,
};

use crate::database::DatabaseInner;
use crate::velesql_scan::{scan_all, OwnedScanRow};
use crate::velesql_value::Params;
use crate::velesql_where;

/// The joined rows of a SELECT with JOIN clauses that its WHERE holds for, in
/// base order.
pub(crate) fn joined_rows(
    db: &DatabaseInner,
    stmt: &SelectStatement,
    params: &Params,
) -> Result<Vec<OwnedScanRow>, String> {
    let from_alias = side_alias(stmt.from_alias.first().map(String::as_str), &stmt.from);
    let mut rows: Vec<OwnedScanRow> = scan_all(db, &stmt.from, None, params)?
        .into_iter()
        .map(|(id, score, payload)| (id, score, Some(scope_base(payload, from_alias, id))))
        .collect();
    for join in &stmt.joins {
        rows = apply_join(db, rows, join, params)?;
    }
    let Some(cond) = &stmt.where_clause else {
        return Ok(rows);
    };
    let mut kept = Vec::with_capacity(rows.len());
    for row in rows {
        if velesql_where::matches(cond, row.0, row.2.as_ref(), params)? {
            kept.push(row);
        }
    }
    Ok(kept)
}

fn apply_join(
    db: &DatabaseInner,
    accumulator: Vec<OwnedScanRow>,
    join: &JoinClause,
    params: &Params,
) -> Result<Vec<OwnedScanRow>, String> {
    reject_unsupported_join(join)?;
    let alias = side_alias(join.alias.as_deref(), &join.table).to_string();
    let right_rows = scan_all(db, &join.table, None, params)?;
    let (left_key, right_key) = equality_keys(join, &alias)?;

    let mut out: Vec<OwnedScanRow> = Vec::new();
    for left in &accumulator {
        join_one_left_row(
            left,
            &right_rows,
            &alias,
            &left_key,
            &right_key,
            join,
            &mut out,
        );
    }
    Ok(out)
}

fn join_one_left_row(
    left: &OwnedScanRow,
    right_rows: &[OwnedScanRow],
    alias: &str,
    left_key: &str,
    right_key: &str,
    join: &JoinClause,
    out: &mut Vec<OwnedScanRow>,
) {
    let (id, score, payload) = left;
    let mut matched_any = false;
    for right in right_rows {
        if rows_match(left, right, left_key, right_key, alias) {
            let empty = serde_json::Map::new();
            let fields = right
                .2
                .as_ref()
                .and_then(|p| p.as_object())
                .unwrap_or(&empty);
            let merged = merge_side(payload.clone(), alias, fields, Some(right.0));
            out.push((*id, *score, Some(merged)));
            matched_any = true;
        }
    }
    if !matched_any && matches!(join.join_type, JoinType::Left) {
        let merged = merge_side(payload.clone(), alias, &serde_json::Map::new(), None);
        out.push((*id, *score, Some(merged)));
    }
}

fn reject_unsupported_join(join: &JoinClause) -> Result<(), String> {
    match join.join_type {
        JoinType::Inner | JoinType::Left => Ok(()),
        JoinType::Right => Err("RIGHT JOIN is not supported in WASM (use LEFT JOIN)".to_string()),
        JoinType::Full => Err("FULL JOIN is not supported in WASM".to_string()),
        _ => Err(format!(
            "Unsupported JOIN type in WASM: {:?}",
            join.join_type
        )),
    }
}

/// Extracts the "ON a.x = b.x" equality keys, normalizing column names so
/// the matcher always compares "unqualified column name on left side" vs
/// "alias.column on right side".
fn equality_keys(join: &JoinClause, alias: &str) -> Result<(String, String), String> {
    if let Some(cond) = &join.condition {
        // Orient the condition by which side targets the joined table, the
        // same way core's `normalize_join_condition` does (issue #1555):
        // the base-side ref is looked up on the accumulated row, the
        // join-side ref on the scanned right row — regardless of which side
        // of `ON` named which table. The joined table is addressable by its
        // alias or by its raw table name even when aliased.
        let is_join_side = |r: &velesdb_core::velesql::ColumnRef| {
            r.table
                .as_deref()
                .is_some_and(|t| t == alias || t == join.table)
        };
        let (base_ref, join_ref) = if !is_join_side(&cond.left) || is_join_side(&cond.right) {
            (&cond.left, &cond.right)
        } else {
            (&cond.right, &cond.left)
        };
        return Ok((key_of(base_ref, alias), key_of(join_ref, alias)));
    }
    if let Some(cols) = &join.using_columns {
        if let Some(first) = cols.first() {
            return Ok((first.clone(), format!("{alias}.{first}")));
        }
    }
    Err("JOIN requires an ON or USING clause in WASM".to_string())
}

fn key_of(ref_: &velesdb_core::velesql::ColumnRef, join_alias: &str) -> String {
    match &ref_.table {
        Some(t) if t == join_alias => format!("{t}.{}", ref_.column),
        Some(t) => format!("{t}.{}", ref_.column),
        None => ref_.column.clone(),
    }
}

fn rows_match(
    left: &OwnedScanRow,
    right: &OwnedScanRow,
    left_key: &str,
    right_key: &str,
    right_alias: &str,
) -> bool {
    // `id` is the base point's id, as in every clause.
    let left_val = if left_key == "id" {
        Some(serde_json::json!(left.0))
    } else {
        left.2
            .as_ref()
            .and_then(|p| p.as_object())
            .and_then(|values| lookup_join_key(values, left_key))
    };
    let right_val = extract_right_value(right, right_key, right_alias);
    match (left_val, right_val) {
        (Some(a), Some(b)) => crate::velesql_value::json_values_equal(&a, &b),
        _ => false,
    }
}

/// Looks up a (possibly alias-qualified) key in the joined row's flat/nested
/// map. `"users.id"` walks `users → id` via the nested alias object; `"id"`
/// falls back to the flat root. Keeps the lookup symmetric with the WHERE
/// evaluator's dotted-path reading (core's `payload_path`).
fn lookup_join_key(
    values: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Option<serde_json::Value> {
    if let Some((head, tail)) = key.split_once('.') {
        if let Some(nested) = values.get(head) {
            if let Some(v) = payload_path(nested, tail) {
                return Some(v.clone());
            }
        }
    }
    values.get(key).cloned()
}

fn extract_right_value(row: &OwnedScanRow, key: &str, alias: &str) -> Option<serde_json::Value> {
    let (id, _score, payload) = row;
    let col = key.strip_prefix(&format!("{alias}.")).unwrap_or(key);
    if col == "id" {
        return Some(serde_json::json!(*id));
    }
    payload.as_ref().and_then(|p| payload_path(p, col).cloned())
}

#[cfg(test)]
#[path = "velesql_join_tests.rs"]
mod tests;
