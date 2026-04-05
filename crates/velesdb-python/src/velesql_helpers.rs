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
        query.dml_collection_name().map(String::from)
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
    pub(crate) fn condition_has_vector_search(cond: &velesdb_core::velesql::Condition) -> bool {
        cond.has_vector_search()
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
