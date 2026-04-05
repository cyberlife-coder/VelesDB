//! Internal helpers for `VelesQL` parsed query introspection.
//!
//! Delegates to core `Query::dml_collection_name()` and
//! `Condition::has_vector_search()` to avoid duplication.

/// Extracts the collection name from a DML statement, if present.
pub(crate) fn dml_collection_name(query: &velesdb_core::velesql::Query) -> Option<String> {
    query.dml_collection_name().map(String::from)
}

/// Recursively check if a condition contains vector search.
pub(crate) fn condition_has_vector_search(cond: &velesdb_core::velesql::Condition) -> bool {
    cond.has_vector_search()
}
