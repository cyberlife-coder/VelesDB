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
fn truncate(db: &mut DatabaseInner, stmt: &TruncateStatement) -> Result<(), String> {
    let summary = db
        .collection_summaries()
        .into_iter()
        .find(|(name, _, _)| name == &stmt.collection)
        .ok_or_else(|| format!("Collection '{}' not found", stmt.collection))?;
    let (_, dim, is_metadata) = summary;
    let metric_name = metric_for_existing(db, &stmt.collection)?;
    db.delete_collection(&stmt.collection)?;
    if is_metadata {
        db.create_metadata_collection(&stmt.collection)
    } else {
        db.create_collection(&stmt.collection, dim, &metric_name)
    }
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

/// Reads back the metric of an existing collection as a canonical string.
fn metric_for_existing(db: &DatabaseInner, name: &str) -> Result<String, String> {
    let handle = db.get_shared_store(name)?;
    let borrowed = handle.borrow();
    Ok(metric_to_string(borrowed.metric))
}

fn metric_to_string(m: velesdb_core::DistanceMetric) -> String {
    use velesdb_core::DistanceMetric;
    match m {
        DistanceMetric::Cosine => "cosine",
        DistanceMetric::Euclidean => "euclidean",
        DistanceMetric::DotProduct => "dot",
        DistanceMetric::Hamming => "hamming",
        DistanceMetric::Jaccard => "jaccard",
        _ => "cosine",
    }
    .to_string()
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
