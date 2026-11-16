//! The row a JOIN produces, as every engine builds it — core's join stages
//! (lookup, indexed, column-store) and the WASM executor alike.
//!
//! A joined row is the base point (its id and score) whose payload carries
//! each side twice:
//!
//! - **flat**, every field at the root (a later side overwriting an earlier
//!   one of the same name), so a bare `amount` reads as before;
//! - **under its alias** (the table name when it has none), with the side's
//!   `id`, so `o.amount`, `c.name` or `c.id` read that side whatever the
//!   others hold — the form a self-join (`e.name`, `m.name`) needs.
//!
//! A LEFT JOIN row without a match carries the alias with a `null` id (and
//! `null` for the columns the join side is known to have), so
//! `WHERE c.id IS NULL` finds it. WHERE, ORDER BY and the projection then read
//! `alias.column` as the nested path it is.

use serde_json::{Map, Value};

/// The alias a FROM or JOIN side is read under: its alias, else its table.
#[must_use]
pub fn side_alias<'a>(alias: Option<&'a str>, table: &'a str) -> &'a str {
    alias.unwrap_or(table)
}

/// The payload of a base row before any join: its own fields, and the same
/// fields with its `id` under `alias`.
#[must_use]
pub fn scope_base(payload: Option<Value>, alias: &str, id: u64) -> Value {
    let mut row = into_object(payload);
    let scoped = scoped(&row, Some(id));
    row.insert(alias.to_string(), scoped);
    Value::Object(row)
}

/// `row` with one joined side merged in: `fields` flat and, with `id`, under
/// `alias`. A side without a match passes the columns it nulls (or none) and
/// no `id`.
#[must_use]
pub fn merge_side(
    row: Option<Value>,
    alias: &str,
    fields: &Map<String, Value>,
    id: Option<u64>,
) -> Value {
    let mut row = into_object(row);
    for (key, value) in fields {
        row.insert(key.clone(), value.clone());
    }
    row.insert(alias.to_string(), scoped(fields, id));
    Value::Object(row)
}

/// The fields of a side under its alias, with its `id`: the one given, else
/// the side's own `id` column, else `null`.
fn scoped(fields: &Map<String, Value>, id: Option<u64>) -> Value {
    let mut side = fields.clone();
    match id {
        Some(id) => {
            side.insert("id".to_string(), Value::from(id));
        }
        None => {
            side.entry("id").or_insert(Value::Null);
        }
    }
    Value::Object(side)
}

/// A payload as an object: a non-object payload contributes no field.
fn into_object(payload: Option<Value>) -> Map<String, Value> {
    match payload {
        Some(Value::Object(map)) => map,
        _ => Map::new(),
    }
}

#[cfg(test)]
#[path = "join_row_tests.rs"]
mod tests;
