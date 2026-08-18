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
    execute(&mut db, &parse_ddl("DROP COLLECTION IF EXISTS ghost")).expect("test: drop if exists");
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
    let rows = execute(&mut db, &parse_ddl("CREATE INDEX ON docs (category)")).expect("test: idx");
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
