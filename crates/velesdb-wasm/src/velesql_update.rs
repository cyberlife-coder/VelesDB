//! UPDATE dispatch for the WASM VelesQL executor (S4-13).
//!
//! Core's UPDATE, over the in-memory store: once the collection resolves,
//! the WHERE's shape is checked by core's rule
//! ([`velesdb_core::velesql::check_dml_where`], a vector, graph or full-text
//! predicate refused with `VELES-010` before any `$name` is bound), then
//! the `SET` list is bound by core's DML binder
//! ([`velesdb_core::velesql::bind_update_assignments`]) before any row is
//! read, so an unbound `$name` refuses the statement whatever its
//! WHERE matches; the WHERE (including IN / BETWEEN / LIKE / AND / OR)
//! selects the rows; and each matched row is rewritten by core's rule
//! ([`velesdb_core::velesql::BoundAssignments::apply`]) — `SET vector = $v`
//! replaces a vector collection's embedding and is refused per row on a
//! metadata-only one, every other column is a payload field. A WHERE that
//! matches no row updates none. All or nothing: every row is rewritten and
//! checked before any is written.

use velesdb_core::velesql::{BoundAssignments, UpdateStatement, VectorColumn};
use velesdb_core::Point;

use crate::database::DatabaseInner;
use crate::exec_error::ExecError;
use crate::store_state::StoreState;
use crate::velesql_helpers::collect_matching_indices;
use crate::velesql_value::Params;

/// Executes an UPDATE statement. Returns the number of rows it rewrote.
pub(crate) fn execute(
    db: &DatabaseInner,
    stmt: &UpdateStatement,
    params: &Params,
) -> Result<u32, ExecError> {
    // Core's order: the collection first (a missing one is VELES-002 even
    // when a `$name` is unbound too), then the `SET` list.
    let store = db.get_shared_store(&stmt.table)?;
    velesdb_core::velesql::check_dml_where("UPDATE", stmt.where_clause.as_ref())?;
    let assignments = velesdb_core::velesql::bind_update_assignments(&stmt.assignments, params)?;
    let indices = collect_matching_indices(&store, stmt.where_clause.as_ref(), params)?;
    let column = crate::store_search::query_vector_target(&stmt.table, store.borrow().dimension)
        .vector_column(false);
    let updated = updated_rows(&store.borrow(), &indices, &assignments, column)?;
    write_rows(&mut store.borrow_mut(), &indices, updated);
    Ok(u32::try_from(indices.len()).unwrap_or(u32::MAX))
}

/// Writes each rewritten row back at its index: its vector (none on a
/// metadata-only store) and its payload.
fn write_rows(store: &mut StoreState, indices: &[usize], rows: Vec<Point>) {
    let dimension = store.dimension;
    for (&idx, point) in indices.iter().zip(rows) {
        if dimension > 0 {
            store.data[idx * dimension..(idx + 1) * dimension].copy_from_slice(&point.vector);
        }
        if let Some(slot) = store.payloads.get_mut(idx) {
            *slot = point.payload;
        }
    }
}

/// The rows at `indices` with `assignments` applied, each vector checked as
/// the store checks an ingest (dimension, finite components).
fn updated_rows(
    store: &StoreState,
    indices: &[usize],
    assignments: &BoundAssignments,
    column: VectorColumn<'_>,
) -> Result<Vec<Point>, ExecError> {
    let dimension = store.dimension;
    indices
        .iter()
        .map(|&idx| {
            let point = stored_point(store, idx);
            let updated = assignments.apply(&point, column)?;
            if dimension > 0 {
                crate::store_search::check_ingest_vector(&updated.vector, dimension)?;
            }
            Ok(updated)
        })
        .collect()
}

/// The row at `idx` as a point: its id, vector (empty on a metadata-only
/// store) and payload.
fn stored_point(store: &StoreState, idx: usize) -> Point {
    let dimension = store.dimension;
    let vector = store
        .data
        .get(idx * dimension..(idx + 1) * dimension)
        .map(<[f32]>::to_vec)
        .unwrap_or_default();
    Point {
        id: store.ids[idx],
        vector,
        payload: store.payloads.get(idx).cloned().flatten(),
        sparse_vectors: None,
    }
}

#[cfg(test)]
#[path = "velesql_update_tests.rs"]
mod tests;
