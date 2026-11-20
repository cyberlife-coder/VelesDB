//! The params blob of the VelesQL executor (S4-13): the `$name` → JSON map
//! a statement binds against. Every `$name` binds through core's binders
//! (`velesdb_core::velesql::bind_value`, `bind_condition`, `bind_vector`,
//! and, for a DML row, `bind_insert_row` / `bind_update_assignments`).

use std::collections::HashMap;

use crate::exec_error::ExecError;

/// Params map for VelesQL queries (`$name` → JSON value).
pub(crate) type Params = HashMap<String, serde_json::Value>;

/// Parses the raw JSON params blob into a [`Params`] map.
///
/// Accepts:
/// - `None` -> empty map
/// - `Some("{}")` -> empty map
/// - `Some("{\"k\": 10}")` -> `{"k" -> 10}`
///
/// Invalid JSON returns a descriptive error string for the FFI layer.
pub(crate) fn parse_params(raw: Option<&str>) -> Result<Params, ExecError> {
    let Some(raw) = raw else {
        return Ok(Params::new());
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(Params::new());
    }
    // The JS surface's own input: core receives its params already decoded.
    serde_json::from_str(trimmed)
        .map_err(|e| ExecError::refused(format!("Invalid params JSON: {e}")))
}

#[cfg(test)]
#[path = "velesql_value_tests.rs"]
mod tests;
