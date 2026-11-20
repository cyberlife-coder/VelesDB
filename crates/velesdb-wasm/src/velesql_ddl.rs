//! DDL dispatch for the WASM VelesQL executor (S4-13).
//!
//! Maps `CREATE COLLECTION`, `DROP COLLECTION`, `TRUNCATE`, `CREATE INDEX`,
//! `DROP INDEX`, and `ANALYZE` AST nodes to the corresponding mutations on
//! [`crate::database::DatabaseInner`]. Index DDL and `ANALYZE` are accepted
//! as no-ops with a descriptive message so JavaScript callers can exercise
//! the full VelesQL surface without crafting branch-specific SQL.

use velesdb_core::velesql::{
    AnalyzeStatement, CreateCollectionKind, CreateCollectionStatement, DdlStatement,
    DropCollectionStatement, TruncateStatement,
};

use crate::database::DatabaseInner;
use crate::exec_error::ExecError;
use crate::velesql_result::QueryResultRow;

/// Executes any supported DDL statement. Index / ANALYZE DDL return a row
/// with a human-readable note so the JS caller sees *something*.
pub(crate) fn execute(
    db: &mut DatabaseInner,
    stmt: &DdlStatement,
) -> Result<Vec<QueryResultRow>, ExecError> {
    match stmt {
        DdlStatement::CreateCollection(s) => create_collection(db, s).map(|()| Vec::new()),
        DdlStatement::DropCollection(s) => drop_collection(db, s).map(|()| Vec::new()),
        DdlStatement::Truncate(s) => truncate(db, s).map(|()| Vec::new()),
        DdlStatement::Analyze(s) => analyze(db, s).map(|row| vec![row]),
        other => note_or_refuse(other).map(|row| vec![row]),
    }
}

/// The DDL WASM does not run: index DDL is accepted as a no-op with a note
/// row; anything else is refused.
fn note_or_refuse(stmt: &DdlStatement) -> Result<QueryResultRow, ExecError> {
    match stmt {
        DdlStatement::CreateIndex(s) => index_noop_row("CREATE INDEX", &s.collection, &s.field),
        DdlStatement::DropIndex(s) => index_noop_row("DROP INDEX", &s.collection, &s.field),
        DdlStatement::AlterCollection(_) => Err(ExecError::refused(
            "ALTER COLLECTION is not supported in WASM yet",
        )),
        // Defensive: DdlStatement is #[non_exhaustive].
        _ => Err(ExecError::refused(format!(
            "Unsupported DDL variant in WASM: {stmt:?}"
        ))),
    }
}

/// Creates either a metadata-only or vector collection.
fn create_collection(
    db: &mut DatabaseInner,
    stmt: &CreateCollectionStatement,
) -> Result<(), ExecError> {
    match &stmt.kind {
        CreateCollectionKind::Metadata => Ok(db.create_metadata_collection(&stmt.name)?),
        CreateCollectionKind::Vector(params) => {
            // Read by core's DDL rule: an unknown metric or mode is core's
            // `Error::Query`. `m` / `ef_construction` tune an HNSW graph WASM
            // does not build.
            let metric = velesdb_core::velesql::resolve_metric(&params.metric)?;
            let mode = velesdb_core::velesql::resolve_storage_mode(params.storage.as_deref())?;
            db.create_collection_with_mode(
                &stmt.name,
                params.dimension,
                metric,
                crate::parsing::core_to_wasm_storage_mode(mode),
            )
        }
        CreateCollectionKind::Graph(_) => Err(ExecError::refused(
            "Graph collections are not supported in WASM (use GraphStore directly)",
        )),
        _ => Err(ExecError::refused(format!(
            "Unsupported CREATE COLLECTION kind in WASM: {:?}",
            stmt.kind
        ))),
    }
}

/// Drops the named collection; `IF EXISTS` swallows "not found" errors.
fn drop_collection(
    db: &mut DatabaseInner,
    stmt: &DropCollectionStatement,
) -> Result<(), ExecError> {
    match db.delete_collection(&stmt.name) {
        Ok(()) => Ok(()),
        Err(_) if stmt.if_exists => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// `TRUNCATE` removes all rows but preserves the collection definition.
/// Any associated graph store (same name) is cleared in-place so ghost
/// nodes/edges cannot survive a TRUNCATE (Devin review PR #594 #3).
///
/// # Handle semantics (Devin Review Finding F12)
///
/// TRUNCATE clears the existing [`StoreState`](crate::store_state::StoreState) through the shared `Rc`
/// instead of dropping and recreating the collection. This guarantees
/// that any `WasmCollectionHandle` obtained *before* the TRUNCATE still
/// observes the emptied store (its `Rc` has not been swapped out).
///
/// Storage mode (Full / SQ8 / Binary / PQ / RaBitQ) and metric are
/// therefore preserved mechanically: the backing struct is the same.
/// Previously (pre-F12) we dropped+recreated, which forced us to round-
/// trip through `read_existing_collection_state` and silently orphaned
/// any outstanding handle.
///
/// DROP COLLECTION retains the old "invalidate-by-drop" semantics — it
/// removes the entry from `collections` so the old `Rc` is no longer
/// reachable via `get_collection`. A subsequent `CREATE` with the same
/// name creates a *new* store that is distinct from the dropped one.
fn truncate(db: &mut DatabaseInner, stmt: &TruncateStatement) -> Result<(), ExecError> {
    let handle = db.get_shared_store(&stmt.collection)?;
    // Clear the inner store in place. This wipes ids, data buffers,
    // payloads, and any sparse index; dimension/metric/storage_mode
    // are preserved on the struct.
    handle.borrow_mut().clear();
    // Graph store lives in a separate map; clear it too so graph DML
    // issued against the same name does not survive the truncate.
    db.clear_graph_store(&stmt.collection);
    Ok(())
}

/// `ANALYZE` returns synthetic statistics about the target collection.
fn analyze(db: &DatabaseInner, stmt: &AnalyzeStatement) -> Result<QueryResultRow, ExecError> {
    let store = db.get_shared_store(&stmt.collection)?;
    let borrowed = store.borrow();
    let payload = serde_json::json!({
        "collection": stmt.collection,
        "row_count": borrowed.ids.len(),
        "dimension": borrowed.dimension(),
        "note": "WASM ANALYZE is synthetic — no cost-based optimizer is available.",
    });
    QueryResultRow::synthetic(payload)
}

/// Synthesises a result row for accepted-but-no-op index DDL so callers
/// know the statement parsed and what the WASM backend actually does.
fn index_noop_row(op: &str, collection: &str, field: &str) -> Result<QueryResultRow, ExecError> {
    let payload = serde_json::json!({
        "op": op,
        "collection": collection,
        "field": field,
        "status": "accepted-noop",
        "note": "WASM uses brute-force scan; INDEX DDL is a no-op but accepted for API parity.",
    });
    QueryResultRow::synthetic(payload)
}

// Note: `metric_to_string` (Finding L DDL guard) was removed alongside
// the delete+recreate TRUNCATE path (Finding F12). TRUNCATE now clears
// the backing store in place, so we no longer serialize the metric back
// to its string form — the metric stays on the struct. DESCRIBE names a
// metric through core's `DistanceMetric::canonical_name`, whose exhaustive
// match is where a new variant gets its name.

#[cfg(test)]
#[path = "velesql_ddl_tests.rs"]
mod tests;
