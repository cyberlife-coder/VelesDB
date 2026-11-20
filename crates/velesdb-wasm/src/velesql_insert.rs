//! INSERT / UPSERT dispatch for the WASM VelesQL executor (S4-13).
//!
//! Each `VALUES` row is bound by core's DML binder
//! ([`velesdb_core::velesql::bind_insert_row`]) into the point core's
//! `INSERT` writes, so the cells accepted, the point built and the words a
//! refusal uses are core's: an `id` that is a non-negative integer, a
//! `vector` bound to a `$param` (no literal is a vector) and required on a
//! vector collection, refused on a metadata-only one, every other column a
//! payload field. The vector is then checked as the store checks an ingest
//! (dimension, finite components).

use velesdb_core::velesql::InsertStatement;

use crate::database::DatabaseInner;
use crate::exec_error::ExecError;
use crate::velesql_value::Params;

/// Executes an INSERT or UPSERT statement. Returns the number of rows
/// upserted (INSERT with duplicate ID counts as an upsert in WASM because the
/// underlying `VectorStore` already replaces on duplicate).
///
/// All or nothing, as core's single batch upsert: every row is bound and
/// checked first, and only a statement whose rows all pass writes any.
pub(crate) fn execute(
    db: &DatabaseInner,
    stmt: &InsertStatement,
    params: &Params,
) -> Result<u32, ExecError> {
    let store = db.get_shared_store(&stmt.table)?;
    let dimension = store.borrow().dimension();
    let points = stmt
        .rows
        .iter()
        .map(|row| bound_point(stmt, row, params, dimension))
        .collect::<Result<Vec<_>, ExecError>>()?;

    let mut borrowed = store.borrow_mut();
    for point in &points {
        if dimension == 0 {
            insert_metadata_row(&mut borrowed, point.id, point.payload.clone());
        } else {
            crate::store_insert::insert_with_payload(
                &mut borrowed,
                point.id,
                &point.vector,
                point.payload.clone(),
            );
        }
    }
    Ok(u32::try_from(points.len()).unwrap_or(u32::MAX))
}

/// One row as the point core's `INSERT` writes, its vector checked as the
/// store checks an ingest. `dimension` 0 is a metadata-only collection; the
/// collections of this executor are no graphs.
fn bound_point(
    stmt: &InsertStatement,
    row: &[velesdb_core::velesql::Value],
    params: &Params,
    dimension: usize,
) -> Result<velesdb_core::Point, ExecError> {
    let column =
        crate::store_search::query_vector_target(&stmt.table, dimension).vector_column(false);
    let point =
        velesdb_core::velesql::bind_insert_row(&stmt.columns, row, params)?.into_point(column)?;
    if dimension > 0 {
        crate::store_search::check_ingest_vector(&point.vector, dimension)?;
    }
    Ok(point)
}

/// Inserts a row into a metadata-only collection (no vector data).
fn insert_metadata_row(
    borrowed: &mut crate::store_state::StoreState,
    id: u64,
    payload: Option<serde_json::Value>,
) {
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
