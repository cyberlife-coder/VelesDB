//! The `recall_where` column filters, from Python tuples to engine filters.

use crate::utils::python_to_json;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use velesdb_memory::{ColumnFilter, ColumnOp};

/// Parse a column-filter operator token (`eq`/`ne`/`lt`/`le`/`gt`/`ge`).
fn parse_op(op: &str) -> PyResult<ColumnOp> {
    match op.to_ascii_lowercase().as_str() {
        "eq" => Ok(ColumnOp::Eq),
        "ne" => Ok(ColumnOp::Ne),
        "lt" => Ok(ColumnOp::Lt),
        "le" => Ok(ColumnOp::Le),
        "gt" => Ok(ColumnOp::Gt),
        "ge" => Ok(ColumnOp::Ge),
        other => Err(PyValueError::new_err(format!(
            "unknown filter op '{other}' (expected eq/ne/lt/le/gt/ge)"
        ))),
    }
}

/// The Python `[(field, op, value)]` tuple list of `recall_where` as engine
/// [`ColumnFilter`]s. The first unknown operator (or unconvertible value)
/// aborts the whole conversion, so a typo never silently degrades into a
/// narrower filter set.
pub(super) fn to_column_filters(
    py: Python<'_>,
    filters: Vec<(String, String, Py<PyAny>)>,
) -> PyResult<Vec<ColumnFilter>> {
    filters
        .into_iter()
        .map(|(field, op, value)| {
            Ok(ColumnFilter {
                field,
                op: parse_op(&op)?,
                value: python_to_json(py, &value)?,
            })
        })
        .collect()
}
