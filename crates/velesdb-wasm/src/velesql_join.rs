//! JOIN execution for the WASM VelesQL executor (S4-13).
//!
//! Implements INNER JOIN and LEFT JOIN via a nested-loop algorithm — fine
//! for WASM datasets which are typically small (< 100K rows). Supports
//! only equality predicates (`ON a.col = b.col`), which covers the 95 %
//! case and keeps the executor readable.
//!
//! Row layout in the join pipeline (see [`JoinedRow::values`]):
//! 1. A **flat mirror** of both tables' columns at the root (last-writer
//!    wins on collision — the alias-qualified form below is the
//!    unambiguous way to disambiguate, same as standard SQL).
//! 2. A **nested object per alias** so alias-qualified references like
//!    `WHERE orders.total > 40` traverse into `orders → total` via
//!    [`crate::filter::get_nested_field`]'s split-on-dot navigation.
//!
//! The two shapes coexist so bare columns (`WHERE total > 40`) still
//! work while alias-qualified refs no longer silently fail.

use velesdb_core::velesql::{JoinClause, JoinType, SelectStatement};

use crate::database::DatabaseInner;
use crate::velesql_result::QueryResultRow;
use crate::velesql_scan::{scan_all, OwnedScanRow};
use crate::velesql_value::Params;
use crate::velesql_where;

/// Executes a SELECT with one or more JOIN clauses. Applies the WHERE
/// clause as a post-filter on the joined row set (so WHERE can reference
/// left or right columns uniformly).
pub(crate) fn execute(
    db: &DatabaseInner,
    stmt: &SelectStatement,
    params: &Params,
) -> Result<Vec<QueryResultRow>, String> {
    let left_rows = scan_all(db, &stmt.from, None, params)?;
    let mut accumulator: Vec<JoinedRow> = left_rows
        .into_iter()
        .map(|r| JoinedRow::from_left(r, &stmt.from))
        .collect();

    for join in &stmt.joins {
        accumulator = apply_join(db, accumulator, join, params)?;
    }

    project_rows(&accumulator, stmt, params)
}

/// A row in the join pipeline. `values` maps every visible column (prefixed
/// by alias when needed) to its JSON value. `id` carries the left-side id
/// for display; subsequent joins don't change the id.
struct JoinedRow {
    id: u64,
    values: serde_json::Map<String, serde_json::Value>,
}

impl JoinedRow {
    fn from_left(row: OwnedScanRow, default_alias: &str) -> Self {
        let (id, _score, payload) = row;
        let mut values = serde_json::Map::new();
        if let Some(serde_json::Value::Object(obj)) = &payload {
            for (k, v) in obj {
                values.insert(k.clone(), v.clone());
            }
        }
        values.insert("id".to_string(), serde_json::json!(id));
        // Alias-scoped nested mirror: `WHERE users.name = ...` navigates
        // `users → name` through `get_nested_field`. The nested object
        // always carries the node id so `WHERE users.id = ...` resolves
        // even when the payload itself has no `id` key.
        values.insert(
            default_alias.to_string(),
            nest_payload_with_id(payload.as_ref(), id),
        );
        Self { id, values }
    }

    fn merge_right(&self, right_row: &OwnedScanRow, right_alias: &str) -> Self {
        let mut values = self.values.clone();
        let (right_id, _score, right_payload) = right_row;
        if let Some(serde_json::Value::Object(obj)) = right_payload {
            for (k, v) in obj {
                // Flat mirror: last-writer wins on collision. Documented in
                // the module docstring — use the alias-qualified form to
                // disambiguate, same as standard SQL.
                values.insert(k.clone(), v.clone());
            }
        }
        // Alias-scoped nested mirror for the right side. Overwrites any
        // prior entry (e.g. a null-padded placeholder from LEFT JOIN).
        values.insert(
            right_alias.to_string(),
            nest_payload_with_id(right_payload.as_ref(), *right_id),
        );
        Self {
            id: self.id,
            values,
        }
    }

    fn merge_right_null(&self, right_alias: &str) -> Self {
        let mut values = self.values.clone();
        // Alias-scoped nested mirror for the un-matched right side: the
        // alias exists but its `id` is null. `WHERE foo.id IS NULL` works
        // via `get_nested_field`.
        values.insert(
            right_alias.to_string(),
            serde_json::json!({ "id": serde_json::Value::Null }),
        );
        Self {
            id: self.id,
            values,
        }
    }
}

/// Clones `payload` into an object with the row id injected as `"id"`.
///
/// Used to build the alias-scoped nested mirror in [`JoinedRow`]. A
/// `None` or non-object payload yields `{"id": <id>}` so that
/// `get_nested_field("alias.id")` still works.
fn nest_payload_with_id(payload: Option<&serde_json::Value>, id: u64) -> serde_json::Value {
    let mut obj = match payload {
        Some(serde_json::Value::Object(map)) => map.clone(),
        _ => serde_json::Map::new(),
    };
    obj.insert("id".to_string(), serde_json::json!(id));
    serde_json::Value::Object(obj)
}

fn apply_join(
    db: &DatabaseInner,
    accumulator: Vec<JoinedRow>,
    join: &JoinClause,
    params: &Params,
) -> Result<Vec<JoinedRow>, String> {
    reject_unsupported_join(join)?;
    let alias = join.alias.clone().unwrap_or_else(|| join.table.clone());
    let right_rows = scan_all(db, &join.table, None, params)?;
    let (left_key, right_key) = equality_keys(join, &alias)?;

    let mut out: Vec<JoinedRow> = Vec::new();
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
    left: &JoinedRow,
    right_rows: &[OwnedScanRow],
    alias: &str,
    left_key: &str,
    right_key: &str,
    join: &JoinClause,
    out: &mut Vec<JoinedRow>,
) {
    let mut matched_any = false;
    for right in right_rows {
        if rows_match(left, right, left_key, right_key, alias) {
            out.push(left.merge_right(right, alias));
            matched_any = true;
        }
    }
    if !matched_any && matches!(join.join_type, JoinType::Left) {
        out.push(left.merge_right_null(alias));
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
        return Ok((key_of(&cond.left, alias), key_of(&cond.right, alias)));
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
    left: &JoinedRow,
    right: &OwnedScanRow,
    left_key: &str,
    right_key: &str,
    right_alias: &str,
) -> bool {
    let left_val = lookup_join_key(&left.values, left_key);
    let right_val = extract_right_value(right, right_key, right_alias);
    match (left_val, right_val) {
        (Some(a), Some(b)) => crate::velesql_value::json_values_equal(&a, &b),
        _ => false,
    }
}

/// Looks up a (possibly alias-qualified) key in the joined row's flat/nested
/// map. `"users.id"` walks `users → id` via the nested alias object; `"id"`
/// falls back to the flat root. Keeps the lookup symmetric with the WHERE
/// evaluator's `get_nested_field` semantics.
fn lookup_join_key(
    values: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Option<serde_json::Value> {
    if let Some((head, tail)) = key.split_once('.') {
        if let Some(nested) = values.get(head) {
            if let Some(v) = crate::filter::get_nested_field(nested, tail) {
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
    payload
        .as_ref()
        .and_then(|p| crate::filter::get_nested_field(p, col).cloned())
}

fn project_rows(
    rows: &[JoinedRow],
    stmt: &SelectStatement,
    params: &Params,
) -> Result<Vec<QueryResultRow>, String> {
    let mut out = Vec::with_capacity(rows.len());
    let offset = stmt.offset.unwrap_or(0);
    let limit = stmt.limit.unwrap_or(u64::MAX);
    let mut skipped: u64 = 0;
    for row in rows {
        if !where_passes(row, stmt, params)? {
            continue;
        }
        if skipped < offset {
            skipped = skipped.saturating_add(1);
            continue;
        }
        if (out.len() as u64) >= limit {
            break;
        }
        let payload = serde_json::Value::Object(row.values.clone());
        out.push(QueryResultRow::synthetic(payload)?);
    }
    Ok(out)
}

fn where_passes(row: &JoinedRow, stmt: &SelectStatement, params: &Params) -> Result<bool, String> {
    let Some(cond) = &stmt.where_clause else {
        return Ok(true);
    };
    let payload = serde_json::Value::Object(row.values.clone());
    velesql_where::matches(cond, row.id, Some(&payload), params)
}

#[cfg(test)]
mod tests {
    use super::*;
    use velesdb_core::velesql::Parser;

    fn seed(db: &mut DatabaseInner) {
        db.create_metadata_collection("users").expect("test: users");
        let users = db.get_shared_store("users").expect("test: users store");
        let mut ub = users.borrow_mut();
        for (id, name) in [(1u64, "Alice"), (2, "Bob")] {
            ub.ids.push(id);
            ub.payloads.push(Some(serde_json::json!({"name": name})));
        }
        drop(ub);

        db.create_metadata_collection("orders")
            .expect("test: orders");
        let orders = db.get_shared_store("orders").expect("test: orders store");
        let mut ob = orders.borrow_mut();
        for (id, uid, total) in [(10u64, 1u64, 50.0f64), (11, 1, 75.0), (12, 2, 20.0)] {
            ob.ids.push(id);
            ob.payloads
                .push(Some(serde_json::json!({"user_id": uid, "total": total})));
        }
    }

    fn parse(sql: &str) -> SelectStatement {
        Parser::parse(sql).expect("test: parse").select
    }

    #[test]
    fn test_inner_join_equality() {
        let mut db = DatabaseInner::new();
        seed(&mut db);
        let stmt = parse("SELECT * FROM users JOIN orders ON users.id = orders.user_id LIMIT 10");
        let rows = execute(&db, &stmt, &Params::new()).expect("test: join");
        assert_eq!(rows.len(), 3); // alice:2 orders + bob:1
    }

    #[test]
    fn test_left_join_preserves_unmatched_left() {
        let mut db = DatabaseInner::new();
        seed(&mut db);
        // Insert a user with no orders.
        db.create_metadata_collection("lonely")
            .expect("test: lonely");
        // Actually easier: add a 3rd user and verify.
        let users = db.get_shared_store("users").expect("test: users");
        let mut ub = users.borrow_mut();
        ub.ids.push(3);
        ub.payloads.push(Some(serde_json::json!({"name": "Carol"})));
        drop(ub);

        let stmt =
            parse("SELECT * FROM users LEFT JOIN orders ON users.id = orders.user_id LIMIT 10");
        let rows = execute(&db, &stmt, &Params::new()).expect("test: left join");
        // 2 for Alice + 1 for Bob + 1 null-padded for Carol = 4
        assert_eq!(rows.len(), 4);
    }

    #[test]
    fn test_join_with_where_filter() {
        let mut db = DatabaseInner::new();
        seed(&mut db);
        let stmt = parse(
            "SELECT * FROM users JOIN orders ON users.id = orders.user_id WHERE name = 'Alice' LIMIT 10",
        );
        let rows = execute(&db, &stmt, &Params::new()).expect("test: join where");
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn test_join_missing_right_collection_errors() {
        let mut db = DatabaseInner::new();
        db.create_metadata_collection("users").expect("test: users");
        let stmt = parse("SELECT * FROM users JOIN ghost ON users.id = ghost.user_id LIMIT 10");
        let err = execute(&db, &stmt, &Params::new());
        assert!(err.is_err());
    }

    #[test]
    fn test_right_join_is_rejected() {
        let mut db = DatabaseInner::new();
        seed(&mut db);
        let stmt =
            parse("SELECT * FROM users RIGHT JOIN orders ON users.id = orders.user_id LIMIT 10");
        let err = execute(&db, &stmt, &Params::new());
        assert!(err.is_err());
        assert!(err.expect_err("test: err").contains("RIGHT JOIN"));
    }
}
