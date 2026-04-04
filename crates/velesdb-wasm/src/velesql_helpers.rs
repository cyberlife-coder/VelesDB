//! Internal helpers for `VelesQL` parsed query introspection.

/// Extracts the collection name from a DML statement, if present.
pub(crate) fn dml_collection_name(query: &velesdb_core::velesql::Query) -> Option<String> {
    use velesdb_core::velesql::DmlStatement;
    let name = match query.dml.as_ref()? {
        DmlStatement::Insert(s) | DmlStatement::Upsert(s) => &s.table,
        DmlStatement::Update(s) => &s.table,
        DmlStatement::Delete(s) => &s.table,
        DmlStatement::InsertEdge(s) => &s.collection,
        DmlStatement::DeleteEdge(s) => &s.collection,
        DmlStatement::SelectEdges(s) => &s.collection,
        DmlStatement::InsertNode(s) => &s.collection,
    };
    if name.is_empty() {
        None
    } else {
        Some(name.clone())
    }
}

/// Recursively check if a condition contains vector search.
pub(crate) fn condition_has_vector_search(cond: &velesdb_core::velesql::Condition) -> bool {
    use velesdb_core::velesql::Condition;

    match cond {
        Condition::VectorSearch(_) | Condition::VectorFusedSearch { .. } => true,
        Condition::And(left, right) | Condition::Or(left, right) => {
            condition_has_vector_search(left) || condition_has_vector_search(right)
        }
        Condition::Group(inner) | Condition::Not(inner) => condition_has_vector_search(inner),
        _ => false,
    }
}
