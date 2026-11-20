//! Internal helpers for `VelesQL` parsed query introspection.
//!
//! Delegates to core `Query::dml_collection_name()` and
//! `Condition::has_vector_search()` to avoid duplication.

use std::cell::RefCell;
use std::rc::Rc;

use velesdb_core::velesql::Condition;

use crate::exec_error::ExecError;
use crate::store_state::StoreState;
use crate::velesql_value::Params;
use crate::velesql_where;

/// Extracts the collection name from a DML statement, if present.
pub(crate) fn dml_collection_name(query: &velesdb_core::velesql::Query) -> Option<String> {
    query.dml_collection_name().map(String::from)
}

/// Recursively check if a condition contains vector search.
pub(crate) fn condition_has_vector_search(cond: &velesdb_core::velesql::Condition) -> bool {
    cond.has_vector_search()
}

/// Walks the store once and returns indices of rows matching the given
/// optional WHERE condition.
///
/// Shared by UPDATE and DELETE dispatchers. When `where_clause` is `None`,
/// every row is considered a match (caller must decide whether that is
/// valid — DELETE requires WHERE, UPDATE allows omission).
pub(crate) fn collect_matching_indices(
    store: &Rc<RefCell<StoreState>>,
    where_clause: Option<&Condition>,
    params: &Params,
) -> Result<Vec<usize>, ExecError> {
    let borrowed = store.borrow();
    let filter = where_clause
        .map(|cond| velesql_where::WhereFilter::compile(cond, params))
        .transpose()?;
    let mut out = Vec::new();
    for (idx, &id) in borrowed.ids.iter().enumerate() {
        let payload = borrowed.payloads.get(idx).and_then(|p| p.as_ref());
        if filter.as_ref().is_none_or(|f| f.matches(id, payload)) {
            out.push(idx);
        }
    }
    Ok(out)
}
