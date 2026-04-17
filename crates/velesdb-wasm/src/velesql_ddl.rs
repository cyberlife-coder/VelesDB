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
use crate::velesql_result::QueryResultRow;

/// Executes any supported DDL statement. Index / ANALYZE DDL return a row
/// with a human-readable note so the JS caller sees *something*.
pub(crate) fn execute(
    db: &mut DatabaseInner,
    stmt: &DdlStatement,
) -> Result<Vec<QueryResultRow>, String> {
    match stmt {
        DdlStatement::CreateCollection(s) => {
            create_collection(db, s)?;
            Ok(Vec::new())
        }
        DdlStatement::DropCollection(s) => {
            drop_collection(db, s)?;
            Ok(Vec::new())
        }
        DdlStatement::Truncate(s) => {
            truncate(db, s)?;
            Ok(Vec::new())
        }
        DdlStatement::CreateIndex(s) => Ok(vec![index_noop_row(
            "CREATE INDEX",
            &s.collection,
            &s.field,
        )?]),
        DdlStatement::DropIndex(s) => {
            Ok(vec![index_noop_row("DROP INDEX", &s.collection, &s.field)?])
        }
        DdlStatement::Analyze(s) => analyze(db, s).map(|row| vec![row]),
        DdlStatement::AlterCollection(_) => {
            Err("ALTER COLLECTION is not supported in WASM yet".to_string())
        }
        // Defensive: DdlStatement is #[non_exhaustive].
        _ => Err(format!("Unsupported DDL variant in WASM: {stmt:?}")),
    }
}

/// Creates either a metadata-only or vector collection.
fn create_collection(
    db: &mut DatabaseInner,
    stmt: &CreateCollectionStatement,
) -> Result<(), String> {
    match &stmt.kind {
        CreateCollectionKind::Metadata => db.create_metadata_collection(&stmt.name),
        CreateCollectionKind::Vector(params) => {
            db.create_collection(&stmt.name, params.dimension, &params.metric)
        }
        CreateCollectionKind::Graph(_) => {
            Err("Graph collections are not supported in WASM (use GraphStore directly)".to_string())
        }
        _ => Err(format!(
            "Unsupported CREATE COLLECTION kind in WASM: {:?}",
            stmt.kind
        )),
    }
}

/// Drops the named collection; `IF EXISTS` swallows "not found" errors.
fn drop_collection(db: &mut DatabaseInner, stmt: &DropCollectionStatement) -> Result<(), String> {
    match db.delete_collection(&stmt.name) {
        Ok(()) => Ok(()),
        Err(_) if stmt.if_exists => Ok(()),
        Err(e) => Err(e),
    }
}

/// `TRUNCATE` removes all rows but preserves the collection definition.
/// Any associated graph store (same name) is cleared in-place so ghost
/// nodes/edges cannot survive a TRUNCATE (Devin review PR #594 #3).
///
/// Storage mode (Full / SQ8 / Binary / PQ / RaBitQ) is preserved across
/// the re-provisioning step (Devin Review Finding M). Without this,
/// TRUNCATE on a non-Full collection would silently downgrade it to Full.
fn truncate(db: &mut DatabaseInner, stmt: &TruncateStatement) -> Result<(), String> {
    let summary = db
        .collection_summaries()
        .into_iter()
        .find(|(name, _, _)| name == &stmt.collection)
        .ok_or_else(|| format!("Collection '{}' not found", stmt.collection))?;
    let (_, dim, is_metadata) = summary;
    // Read back the current metric + storage mode BEFORE dropping the
    // collection — after `delete_collection` the store is gone.
    let (metric_name, storage_mode) = read_existing_collection_state(db, &stmt.collection)?;
    db.delete_collection(&stmt.collection)?;
    // delete_collection also dropped the graph store; the subsequent
    // create re-provisions the collection, and any further graph DML
    // will lazily re-create a fresh graph store. We still clear any
    // graph store that might have been re-created between the two
    // calls (defensive — currently unreachable but keeps the invariant
    // explicit).
    if is_metadata {
        db.create_metadata_collection(&stmt.collection)?;
    } else {
        db.create_collection_with_mode(&stmt.collection, dim, &metric_name, storage_mode)?;
    }
    db.clear_graph_store(&stmt.collection);
    Ok(())
}

/// Reads back the metric string and typed storage mode of an existing
/// collection. Split from [`truncate`] so the store borrow is released
/// before `delete_collection` re-borrows the map.
fn read_existing_collection_state(
    db: &DatabaseInner,
    name: &str,
) -> Result<(String, crate::StorageMode), String> {
    let handle = db.get_shared_store(name)?;
    let borrowed = handle.borrow();
    let metric_name = metric_to_string(borrowed.metric)?;
    let storage_mode = borrowed.storage_mode_kind();
    Ok((metric_name, storage_mode))
}

/// `ANALYZE` returns synthetic statistics about the target collection.
fn analyze(db: &DatabaseInner, stmt: &AnalyzeStatement) -> Result<QueryResultRow, String> {
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
fn index_noop_row(op: &str, collection: &str, field: &str) -> Result<QueryResultRow, String> {
    let payload = serde_json::json!({
        "op": op,
        "collection": collection,
        "field": field,
        "status": "accepted-noop",
        "note": "WASM uses brute-force scan; INDEX DDL is a no-op but accepted for API parity.",
    });
    QueryResultRow::synthetic(payload)
}

/// Converts a [`velesdb_core::DistanceMetric`] into its canonical VelesQL
/// string form for re-provisioning via
/// [`DatabaseInner::create_collection_with_mode`].
///
/// Fails loud on unknown variants rather than silently defaulting to
/// `"cosine"` (Devin Review Finding L). `DistanceMetric` is
/// `#[non_exhaustive]` — a future variant added in core would otherwise
/// cause TRUNCATE to silently rebuild the collection with the wrong
/// metric. The error surfaces an explicit, actionable failure instead.
fn metric_to_string(m: velesdb_core::DistanceMetric) -> Result<String, String> {
    use velesdb_core::DistanceMetric;
    let s = match m {
        DistanceMetric::Cosine => "cosine",
        DistanceMetric::Euclidean => "euclidean",
        DistanceMetric::DotProduct => "dot",
        DistanceMetric::Hamming => "hamming",
        DistanceMetric::Jaccard => "jaccard",
        // TODO(US-S4-13): update when DistanceMetric gains new variants.
        // Surfacing Err rather than silently rebuilding with cosine.
        _ => {
            return Err(format!(
                "Unknown distance metric {m:?}; refusing to silently default for TRUNCATE"
            ))
        }
    };
    Ok(s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use velesdb_core::velesql::Parser;

    fn parse_ddl(sql: &str) -> DdlStatement {
        let q = Parser::parse(sql).expect("test: parse");
        q.ddl.expect("test: has ddl")
    }

    #[test]
    fn test_create_metadata_collection() {
        let mut db = DatabaseInner::new();
        let rows =
            execute(&mut db, &parse_ddl("CREATE METADATA COLLECTION docs")).expect("test: create");
        assert!(rows.is_empty());
        assert!(db.contains("docs"));
    }

    #[test]
    fn test_create_vector_collection() {
        let mut db = DatabaseInner::new();
        let rows = execute(
            &mut db,
            &parse_ddl("CREATE COLLECTION vecs (dimension = 4, metric = 'cosine')"),
        )
        .expect("test: create");
        assert!(rows.is_empty());
    }

    #[test]
    fn test_drop_if_exists_is_idempotent() {
        let mut db = DatabaseInner::new();
        execute(&mut db, &parse_ddl("DROP COLLECTION IF EXISTS ghost"))
            .expect("test: drop if exists");
    }

    #[test]
    fn test_truncate_preserves_schema_removes_data() {
        let mut db = DatabaseInner::new();
        db.create_collection("vecs", 4, "cosine")
            .expect("test: create");
        let store = db.get_shared_store("vecs").expect("test: store");
        store
            .borrow_mut()
            .insert(1, &[1.0, 0.0, 0.0, 0.0])
            .expect("test: insert");
        drop(store);

        execute(&mut db, &parse_ddl("TRUNCATE vecs")).expect("test: truncate");
        let store = db.get_shared_store("vecs").expect("test: store");
        assert!(store.borrow().is_empty());
    }

    #[test]
    fn test_create_index_is_accepted_as_noop() {
        let mut db = DatabaseInner::new();
        db.create_metadata_collection("docs").expect("test: seed");
        let rows =
            execute(&mut db, &parse_ddl("CREATE INDEX ON docs (category)")).expect("test: idx");
        assert_eq!(rows.len(), 1);
        assert!(rows[0].data_json_ref().contains("accepted-noop"));
    }

    #[test]
    fn test_drop_index_is_accepted_as_noop() {
        let mut db = DatabaseInner::new();
        db.create_metadata_collection("docs").expect("test: seed");
        let rows =
            execute(&mut db, &parse_ddl("DROP INDEX ON docs (category)")).expect("test: drop idx");
        assert_eq!(rows.len(), 1);
        assert!(rows[0].data_json_ref().contains("DROP INDEX"));
    }

    // --- Finding L: metric_to_string round-trip (DDL side) ---------------
    //
    // For every currently-supported variant, metric_to_string must produce
    // a string that `parse_metric_inner` (the DDL side) accepts back. Any
    // future variant added in core without updating these matches surfaces
    // as an Err rather than silently defaulting to "cosine".

    #[test]
    fn test_metric_to_string_roundtrip_for_all_supported_variants() {
        use velesdb_core::DistanceMetric;
        for m in [
            DistanceMetric::Cosine,
            DistanceMetric::Euclidean,
            DistanceMetric::DotProduct,
            DistanceMetric::Hamming,
            DistanceMetric::Jaccard,
        ] {
            let s = metric_to_string(m).expect("supported variant must serialize");
            // The string must round-trip through `create_collection` (which
            // delegates to `parse_metric_inner`).
            let mut db = DatabaseInner::new();
            let name = format!("tmp_{m:?}");
            db.create_collection(&name, 4, &s).unwrap_or_else(|e| {
                panic!("metric_to_string produced unparseable string '{s}': {e}")
            });
        }
    }

    #[test]
    fn test_analyze_returns_synthetic_stats() {
        let mut db = DatabaseInner::new();
        db.create_metadata_collection("docs").expect("test: seed");
        let rows = execute(&mut db, &parse_ddl("ANALYZE docs")).expect("test: analyze");
        assert_eq!(rows.len(), 1);
        assert!(rows[0].data_json_ref().contains("row_count"));
    }

    #[test]
    fn test_analyze_missing_collection_errors() {
        let mut db = DatabaseInner::new();
        let err = execute(&mut db, &parse_ddl("ANALYZE ghost"));
        assert!(err.is_err());
    }
}
