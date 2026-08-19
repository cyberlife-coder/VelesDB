//! DELETE dispatch for the WASM VelesQL executor (S4-13).
//!
//! Evaluates the mandatory WHERE clause against every row of the target
//! collection and removes the matching entries. Works on both metadata-only
//! and vector collections (the store's `swap_remove` path handles both).

use velesdb_core::velesql::DeleteStatement;

use crate::database::DatabaseInner;
use crate::velesql_helpers::collect_matching_indices;
use crate::velesql_value::Params;

/// Executes a DELETE statement. Returns the number of rows removed.
pub(crate) fn execute(
    db: &DatabaseInner,
    stmt: &DeleteStatement,
    params: &Params,
) -> Result<u32, String> {
    let store = db.get_shared_store(&stmt.table)?;
    let to_remove = collect_matching_indices(&store, Some(&stmt.where_clause), params)?;
    remove_indices_desc(&store, &to_remove);

    Ok(u32::try_from(to_remove.len()).unwrap_or(u32::MAX))
}

/// Removes rows from the store in descending order to keep indices stable.
fn remove_indices_desc(
    store: &std::rc::Rc<std::cell::RefCell<crate::vector_store::VectorStore>>,
    indices: &[usize],
) {
    let mut sorted = indices.to_vec();
    sorted.sort_unstable_by(|a, b| b.cmp(a));
    let mut borrowed = store.borrow_mut();
    for &idx in &sorted {
        crate::store_insert::remove_at_index(&mut borrowed, idx);
    }
}

#[cfg(test)]
#[path = "velesql_delete_tests.rs"]
mod tests;
