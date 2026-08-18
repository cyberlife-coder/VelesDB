//! Admin dispatch for the WASM VelesQL executor (S4-13).
//!
//! `FLUSH` is a no-op in WASM because the store is in-memory only (no
//! persistence feature on wasm32). The executor still accepts the statement
//! and returns a `Admin` result with a descriptive message so that
//! cross-target parity with the Mobile executor is preserved.
//!
//! `ANALYZE` is handled by the DDL module (it is a DDL statement upstream),
//! not by this one.

use velesdb_core::velesql::{AdminStatement, FlushStatement};

use crate::database::DatabaseInner;

/// Executes an admin statement. Returns `Ok(message)` on success.
pub(crate) fn execute(db: &DatabaseInner, stmt: &AdminStatement) -> Result<String, String> {
    match stmt {
        AdminStatement::Flush(s) => flush(db, s),
        // Defensive: `AdminStatement` is `#[non_exhaustive]`.
        _ => Err(format!("Unsupported admin variant in WASM: {stmt:?}")),
    }
}

/// Validates the optional target collection and returns a message describing
/// the no-op behaviour.
fn flush(db: &DatabaseInner, stmt: &FlushStatement) -> Result<String, String> {
    if let Some(name) = &stmt.collection {
        if !db.contains(name) {
            return Err(format!("Collection '{name}' not found"));
        }
    }
    Ok("FLUSH is a no-op in WASM (in-memory only)".to_string())
}

#[cfg(test)]
#[path = "velesql_admin_tests.rs"]
mod tests;
