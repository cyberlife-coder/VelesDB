//! Helper functions for VelesQL Python bindings.
//!
//! Contains query type classification, DML collection name extraction,
//! vector search detection, error formatting, and module registration.

use pyo3::prelude::*;
use velesdb_core::velesql::Query;

use super::velesql::{ParsedStatement, VelesQL, VelesQLParameterError, VelesQLSyntaxError};

impl ParsedStatement {
    /// Extracts the collection name from a DML statement, if present.
    pub(crate) fn dml_collection_name(query: &Query) -> Option<String> {
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

    /// Returns a human-readable label for the query type.
    pub(crate) fn query_type_label(&self) -> &'static str {
        if self.inner.is_ddl_query() {
            "DDL"
        } else if self.inner.is_dml_query() {
            "DML"
        } else if self.inner.is_train() {
            "TRAIN"
        } else if self.inner.is_match_query() {
            "MATCH"
        } else {
            "SELECT"
        }
    }

    /// Recursively check if a condition contains vector search.
    pub(crate) fn condition_has_vector_search(
        cond: &velesdb_core::velesql::Condition,
    ) -> bool {
        use velesdb_core::velesql::Condition;

        match cond {
            Condition::VectorSearch(_) | Condition::VectorFusedSearch { .. } => true,
            Condition::And(left, right) | Condition::Or(left, right) => {
                Self::condition_has_vector_search(left)
                    || Self::condition_has_vector_search(right)
            }
            Condition::Group(inner) => Self::condition_has_vector_search(inner),
            Condition::Not(inner) => Self::condition_has_vector_search(inner),
            _ => false,
        }
    }
}

/// Format a parse error for Python exception message.
pub(crate) fn format_parse_error(e: &velesdb_core::velesql::ParseError) -> String {
    format!(
        "VelesQL syntax error at position {}: {} (near '{}')",
        e.position, e.message, e.fragment
    )
}

/// Register VelesQL classes with the Python module.
pub fn register_velesql_module(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<VelesQL>()?;
    m.add_class::<ParsedStatement>()?;
    m.add(
        "VelesQLSyntaxError",
        m.py().get_type::<VelesQLSyntaxError>(),
    )?;
    m.add(
        "VelesQLParameterError",
        m.py().get_type::<VelesQLParameterError>(),
    )?;
    Ok(())
}
