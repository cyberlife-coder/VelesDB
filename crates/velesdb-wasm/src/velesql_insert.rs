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
mod tests {
    use super::*;
    use crate::database::DatabaseInner;
    use crate::velesql_value::parse_params;
    use velesdb_core::velesql::{DmlStatement, Parser};

    fn parse_insert(sql: &str) -> InsertStatement {
        let q = Parser::parse(sql).expect("test: parse");
        match q.dml.expect("test: has dml") {
            DmlStatement::Insert(s) | DmlStatement::Upsert(s) => s,
            other => panic!("expected INSERT/UPSERT, got {other:?}"),
        }
    }

    fn empty_params() -> Params {
        parse_params(None).expect("test: empty params")
    }

    #[test]
    fn test_insert_single_metadata_row() {
        let mut db = DatabaseInner::new();
        db.create_metadata_collection("docs").expect("test: create");
        let stmt = parse_insert("INSERT INTO docs (id, title) VALUES (1, 'hello')");
        let n = execute(&db, &stmt, &empty_params()).expect("test: insert");
        assert_eq!(n, 1);
    }

    #[test]
    fn test_insert_multi_row_metadata() {
        let mut db = DatabaseInner::new();
        db.create_metadata_collection("docs").expect("test: create");
        let stmt = parse_insert("INSERT INTO docs (id, title) VALUES (1, 'a'), (2, 'b'), (3, 'c')");
        let n = execute(&db, &stmt, &empty_params()).expect("test: insert");
        assert_eq!(n, 3);
    }

    #[test]
    fn test_insert_missing_id_column_errors() {
        let mut db = DatabaseInner::new();
        db.create_metadata_collection("docs").expect("test: create");
        let stmt = parse_insert("INSERT INTO docs (title) VALUES ('hello')");
        let err = execute(&db, &stmt, &empty_params());
        assert!(err.is_err());
        assert!(err.expect_err("test: err").contains("'id'"));
    }

    #[test]
    fn test_insert_vector_collection_without_vector_errors() {
        let mut db = DatabaseInner::new();
        db.create_collection("vecs", 4, "cosine")
            .expect("test: create");
        let stmt = parse_insert("INSERT INTO vecs (id, title) VALUES (1, 'x')");
        let err = execute(&db, &stmt, &empty_params());
        assert!(err.is_err());
        assert!(err.expect_err("test: err").contains("vector"));
    }

    #[test]
    fn test_insert_vector_collection_with_param() {
        let mut db = DatabaseInner::new();
        db.create_collection("vecs", 4, "cosine")
            .expect("test: create");
        let stmt = parse_insert("INSERT INTO vecs (id, vector, tag) VALUES (1, $v, 'a')");
        let params =
            parse_params(Some(r#"{"v": [1.0, 0.0, 0.0, 0.0]}"#)).expect("test: parse params");
        let n = execute(&db, &stmt, &params).expect("test: insert");
        assert_eq!(n, 1);
    }

    #[test]
    fn test_insert_vector_dimension_mismatch_errors() {
        let mut db = DatabaseInner::new();
        db.create_collection("vecs", 4, "cosine")
            .expect("test: create");
        let stmt = parse_insert("INSERT INTO vecs (id, vector) VALUES (1, $v)");
        let params = parse_params(Some(r#"{"v": [1.0, 0.0]}"#)).expect("test: parse params");
        let err = execute(&db, &stmt, &params);
        assert!(err.is_err());
        assert!(err.expect_err("test: err").contains("dimension mismatch"));
    }

    #[test]
    fn test_insert_missing_collection_errors() {
        let db = DatabaseInner::new();
        let stmt = parse_insert("INSERT INTO ghost (id, t) VALUES (1, 'x')");
        let err = execute(&db, &stmt, &empty_params());
        assert!(err.is_err());
        assert!(err
            .expect_err("test: err")
            .contains("Collection 'ghost' not found"));
    }

    #[test]
    fn test_upsert_replaces_existing_row() {
        let mut db = DatabaseInner::new();
        db.create_metadata_collection("docs").expect("test: create");

        // First insert
        let stmt = parse_insert("INSERT INTO docs (id, title) VALUES (1, 'first')");
        execute(&db, &stmt, &empty_params()).expect("test: first insert");

        // Second insert with same id — should replace
        let stmt2 = parse_insert("UPSERT INTO docs (id, title) VALUES (1, 'second')");
        execute(&db, &stmt2, &empty_params()).expect("test: upsert");

        // Verify only one row remains
        let store = db.get_shared_store("docs").expect("test: store");
        assert_eq!(store.borrow().len(), 1);
    }

    #[test]
    fn test_insert_rejects_row_arity_mismatch_at_parse_time() {
        // The parser rejects row-arity mismatches, so this is mostly a
        // defensive sanity check for the validate_statement path when a row
        // is manually crafted.
        let mut stmt = parse_insert("INSERT INTO docs (id, title) VALUES (1, 'a')");
        stmt.rows.push(vec![Value::Integer(2)]); // arity mismatch
        let db = DatabaseInner::new();
        let err = execute(&db, &stmt, &empty_params());
        assert!(err.is_err());
    }
}
