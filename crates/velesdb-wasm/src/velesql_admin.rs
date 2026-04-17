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
mod tests {
    use super::*;
    use crate::database::DatabaseInner;
    use velesdb_core::velesql::Parser;

    fn parse_admin(sql: &str) -> AdminStatement {
        let q = Parser::parse(sql).expect("test: parse");
        q.admin.expect("test: has admin")
    }

    #[test]
    fn test_flush_no_collection_is_noop() {
        let db = DatabaseInner::new();
        let msg = execute(&db, &parse_admin("FLUSH")).expect("test: flush");
        assert!(msg.contains("no-op"));
    }

    #[test]
    fn test_flush_full_no_collection_is_noop() {
        let db = DatabaseInner::new();
        let msg = execute(&db, &parse_admin("FLUSH FULL")).expect("test: flush full");
        assert!(msg.contains("no-op"));
    }

    #[test]
    fn test_flush_existing_collection_ok() {
        let mut db = DatabaseInner::new();
        db.create_metadata_collection("docs").expect("test: create");
        let msg = execute(&db, &parse_admin("FLUSH docs")).expect("test: flush docs");
        assert!(msg.contains("no-op"));
    }

    #[test]
    fn test_flush_missing_collection_errors() {
        let db = DatabaseInner::new();
        let err = execute(&db, &parse_admin("FLUSH ghost"));
        assert!(err.is_err());
    }
}
