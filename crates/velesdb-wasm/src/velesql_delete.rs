//! DELETE dispatch for the WASM VelesQL executor (S4-13).
//!
//! Evaluates the mandatory WHERE clause against every row of the target
//! collection and removes the matching entries. Works on both metadata-only
//! and vector collections (the store's `swap_remove` path handles both).

use velesdb_core::velesql::DeleteStatement;

use crate::database::DatabaseInner;
use crate::exec_error::ExecError;
use crate::velesql_helpers::collect_matching_indices;
use crate::velesql_value::Params;

/// Executes a DELETE statement. Returns the number of rows removed.
pub(crate) fn execute(
    db: &DatabaseInner,
    stmt: &DeleteStatement,
    params: &Params,
) -> Result<u32, ExecError> {
    let store = db.get_shared_store(&stmt.table)?;
    // Core's order: the WHERE's shape before any `$name` is bound.
    velesdb_core::velesql::check_dml_where("DELETE", Some(&stmt.where_clause))?;
    let to_remove = collect_matching_indices(&store, Some(&stmt.where_clause), params)?;
    let ids: Vec<u64> = {
        let borrowed = store.borrow();
        to_remove
            .iter()
            .filter_map(|&idx| borrowed.ids.get(idx).copied())
            .collect()
    };
    remove_indices_desc(&store, &to_remove);
    // As core's delete (#900): a deleted point takes its edges with it.
    db.drop_edges_of(&stmt.table, &ids);

    Ok(u32::try_from(to_remove.len()).unwrap_or(u32::MAX))
}

/// Removes rows from the store in descending order to keep indices stable.
fn remove_indices_desc(
    store: &std::rc::Rc<std::cell::RefCell<crate::store_state::StoreState>>,
    indices: &[usize],
) {
    let mut sorted = indices.to_vec();
    sorted.sort_unstable_by(|a, b| b.cmp(a));
    let mut borrowed = store.borrow_mut();
    for &idx in &sorted {
        crate::store_insert::delete_at_index(&mut borrowed, idx);
    }
}

#[cfg(test)]
#[path = "velesql_delete_tests.rs"]
mod tests;
