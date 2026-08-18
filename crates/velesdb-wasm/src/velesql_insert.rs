//! INSERT / UPSERT dispatch for the WASM VelesQL executor (S4-13).
//!
//! Handles single- and multi-row `INSERT` / `UPSERT` statements against a
//! `WasmDatabase` collection. Supports:
//!
//! - Metadata-only collections (no `vector` column required)
//! - Vector collections (require a `vector` column bound to a `$param`)
//! - Multi-row `VALUES (...), (...), (...)` with mixed payload fields
//! - `$param` substitution for scalar payload values AND the `vector` column
//!
//! Vector literal inlining is NOT supported — vectors must be passed via
//! `$param` as a JSON array. This matches the Mobile executor semantics.

use velesdb_core::velesql::{InsertStatement, Value};

use crate::database::DatabaseInner;
use crate::velesql_value::{json_to_f32_vec, resolve_value, Params};

/// Executes an INSERT or UPSERT statement. Returns the number of rows
/// successfully upserted (INSERT with duplicate ID counts as an upsert in
/// WASM because the underlying `VectorStore` already replaces on duplicate).
pub(crate) fn execute(
    db: &DatabaseInner,
    stmt: &InsertStatement,
    params: &Params,
) -> Result<u32, String> {
    validate_statement(stmt)?;
    let store = db.get_shared_store(&stmt.table)?;
    let is_metadata = store.borrow().dimension() == 0;

    let vector_idx = stmt.columns.iter().position(|c| c == "vector");
    let id_idx = find_required_column(&stmt.columns, "id")?;

    if !is_metadata && vector_idx.is_none() {
        return Err(format!(
            "Collection '{}' is a vector collection; INSERT must include a 'vector' column",
            stmt.table
        ));
    }

    let mut inserted: u32 = 0;
    for row in &stmt.rows {
        insert_row(&store, stmt, row, params, id_idx, vector_idx, is_metadata)?;
        inserted = inserted.saturating_add(1);
    }
    Ok(inserted)
}

/// Validates global statement invariants (columns present, rows non-empty,
/// row arity matches columns).
fn validate_statement(stmt: &InsertStatement) -> Result<(), String> {
    if stmt.columns.is_empty() {
        return Err("INSERT requires at least one column".to_string());
    }
    if stmt.rows.is_empty() {
        return Err("INSERT requires at least one VALUES row".to_string());
    }
    for (i, row) in stmt.rows.iter().enumerate() {
        if row.len() != stmt.columns.len() {
            return Err(format!(
                "INSERT row {i} has {} values but {} columns were declared",
                row.len(),
                stmt.columns.len()
            ));
        }
    }
    Ok(())
}

/// Finds the index of a required column or returns a descriptive error.
fn find_required_column(columns: &[String], name: &str) -> Result<usize, String> {
    columns
        .iter()
        .position(|c| c == name)
        .ok_or_else(|| format!("INSERT must include an '{name}' column"))
}

/// Inserts a single row into the store.
fn insert_row(
    store: &std::rc::Rc<std::cell::RefCell<crate::vector_store::VectorStore>>,
    stmt: &InsertStatement,
    row: &[Value],
    params: &Params,
    id_idx: usize,
    vector_idx: Option<usize>,
    is_metadata: bool,
) -> Result<(), String> {
    let id = extract_row_id(row, id_idx, params)?;
    let payload = build_payload(&stmt.columns, row, id_idx, vector_idx, params)?;

    if is_metadata {
        insert_metadata_row(store, id, payload);
        return Ok(());
    }

    // Vector collection path — resolve the vector from its bound parameter.
    let vector = resolve_vector_cell(row, vector_idx, params)?;
    let expected = store.borrow().dimension();
    if vector.len() != expected {
        return Err(format!(
            "Vector dimension mismatch for id {id}: expected {expected}, got {}",
            vector.len()
        ));
    }
    crate::store_insert::insert_with_payload(&mut store.borrow_mut(), id, &vector, payload);
    Ok(())
}

/// Resolves the `id` column of a row into a `u64`.
///
/// Accepts integer literals and `$param`-bound integers; rejects strings,
/// floats (non-integral), and NULL.
fn extract_row_id(row: &[Value], id_idx: usize, params: &Params) -> Result<u64, String> {
    let raw = resolve_value(&row[id_idx], params)?;
    match raw {
        serde_json::Value::Number(n) => n
            .as_u64()
            .or_else(|| n.as_i64().and_then(|i| u64::try_from(i).ok()))
            .ok_or_else(|| "INSERT id must fit in u64".to_string()),
        other => Err(format!("INSERT id must be an integer, got {other}")),
    }
}

/// Resolves the `vector` cell of a row into a `Vec<f32>` via `$param`.
fn resolve_vector_cell(
    row: &[Value],
    vector_idx: Option<usize>,
    params: &Params,
) -> Result<Vec<f32>, String> {
    let idx = vector_idx.ok_or_else(|| {
        "Vector collection INSERT requires a 'vector' column bound to $param".to_string()
    })?;
    match &row[idx] {
        Value::Parameter(name) => {
            let value = params
                .get(name.as_str())
                .ok_or_else(|| format!("Vector parameter ${name} is not bound"))?;
            json_to_f32_vec(value, name.as_str())
        }
        Value::Null => Err("Vector column cannot be NULL".to_string()),
        other => Err(format!(
            "Vector column must be bound via $param (got literal {other:?}); \
             inline vectors are not supported in WASM INSERT"
        )),
    }
}

/// Builds the payload object for a row by projecting all columns except
/// `id` and `vector`.
fn build_payload(
    columns: &[String],
    row: &[Value],
    id_idx: usize,
    vector_idx: Option<usize>,
    params: &Params,
) -> Result<Option<serde_json::Value>, String> {
    let mut map = serde_json::Map::new();
    for (i, col) in columns.iter().enumerate() {
        if i == id_idx {
            continue;
        }
        if Some(i) == vector_idx {
            continue;
        }
        let value = resolve_value(&row[i], params)?;
        map.insert(col.clone(), value);
    }
    if map.is_empty() {
        return Ok(None);
    }
    Ok(Some(serde_json::Value::Object(map)))
}

/// Inserts a row into a metadata-only collection (no vector data).
fn insert_metadata_row(
    store: &std::rc::Rc<std::cell::RefCell<crate::vector_store::VectorStore>>,
    id: u64,
    payload: Option<serde_json::Value>,
) {
    let mut borrowed = store.borrow_mut();
    // Remove any existing row with the same id (upsert semantics).
    if let Some(idx) = borrowed.ids.iter().position(|&x| x == id) {
        borrowed.ids.swap_remove(idx);
        borrowed.payloads.swap_remove(idx);
    }
    borrowed.ids.push(id);
    borrowed.payloads.push(payload);
}

#[cfg(test)]
#[path = "velesql_insert_tests.rs"]
mod tests;
