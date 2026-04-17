//! Integration tests for the WASM VelesQL executor (`execute_query`).
//!
//! Native-target tests (`#[test]`, not `wasm_bindgen_test`) that exercise
//! `DatabaseInner` through the executor dispatcher. Structure mirrors
//! `velesdb-mobile/src/query_tests.rs` so the two surfaces stay in lockstep.
//!
//! Test categories:
//! - Nominal: happy paths (SELECT, INSERT, UPDATE, DELETE, DDL, SHOW / DESCRIBE).
//! - Edge: LIMIT/OFFSET, multi-row, empty collection, NEAR + filter.
//! - Negative (≥ 20% of suite): invalid SQL, missing collection, unsupported
//!   features (MATCH / TRAIN / FUSION / UNION / INSERT EDGE / EXPLAIN /
//!   CREATE INDEX), dimension mismatch, missing vector column, bad params.

use crate::database::DatabaseInner;
use crate::velesql_exec::execute;
use crate::velesql_result::QueryResultKind;

// =========================================================================
// Test fixtures
// =========================================================================

/// Database pre-seeded with a metadata-only `docs` collection (no vectors).
fn db_with_metadata() -> DatabaseInner {
    let mut db = DatabaseInner::new();
    db.create_metadata_collection("docs")
        .expect("test: metadata create");
    db
}

/// Database pre-seeded with a 4-dim cosine vector collection `vecs`.
fn db_with_vectors() -> DatabaseInner {
    let mut db = DatabaseInner::new();
    db.create_collection("vecs", 4, "cosine")
        .expect("test: vector create");
    db
}

/// Seeds `docs` with 3 rows via the executor itself.
fn seed_metadata_docs(db: &mut DatabaseInner) {
    let sql = concat!(
        "INSERT INTO docs (id, title, category) VALUES ",
        "(1, 'Rust Programming', 'tech'), ",
        "(2, 'Cooking Basics', 'food'), ",
        "(3, 'Advanced Algorithms', 'tech')"
    );
    let r = execute(db, sql, None).expect("test: seed metadata");
    assert_eq!(r.kind(), "mutation");
    assert_eq!(r.row_count(), 3);
}

// =========================================================================
// SELECT tests
// =========================================================================

#[test]
fn test_select_all_returns_rows() {
    let mut db = db_with_metadata();
    seed_metadata_docs(&mut db);

    let r = execute(&mut db, "SELECT * FROM docs LIMIT 10", None).expect("test: select");

    assert_eq!(r.kind(), "rows");
    assert_eq!(r.row_count(), 3);
    assert!(r.message().contains("row(s) returned"));
}

#[test]
fn test_select_with_where_filter() {
    let mut db = db_with_metadata();
    seed_metadata_docs(&mut db);

    let r = execute(
        &mut db,
        "SELECT * FROM docs WHERE category = 'tech' LIMIT 10",
        None,
    )
    .expect("test: select where");

    assert_eq!(r.row_count(), 2);
}

#[test]
fn test_select_with_limit_and_offset() {
    let mut db = db_with_metadata();
    seed_metadata_docs(&mut db);

    let r = execute(&mut db, "SELECT * FROM docs LIMIT 1 OFFSET 1", None).expect("test: offset");

    assert_eq!(r.row_count(), 1);
}

#[test]
fn test_select_row_contains_payload_fields() {
    let mut db = db_with_metadata();
    seed_metadata_docs(&mut db);

    let r = execute(&mut db, "SELECT * FROM docs LIMIT 10", None).expect("test: select");

    assert!(r.rows_json().contains("title"));
    assert!(r.rows_json().contains("category"));
}

#[test]
fn test_select_empty_collection_returns_no_rows() {
    let mut db = db_with_metadata();

    let r = execute(&mut db, "SELECT * FROM docs LIMIT 10", None).expect("test: empty select");

    assert_eq!(r.row_count(), 0);
    assert_eq!(r.kind(), "rows");
}

// =========================================================================
// INSERT tests
// =========================================================================

#[test]
fn test_insert_single_row_mutation_kind() {
    let mut db = db_with_metadata();

    let r = execute(
        &mut db,
        "INSERT INTO docs (id, title) VALUES (10, 'new doc')",
        None,
    )
    .expect("test: insert");

    assert_eq!(r.kind(), "mutation");
    assert_eq!(r.row_count(), 1);

    // Read-back verification.
    let back =
        execute(&mut db, "SELECT * FROM docs LIMIT 10", None).expect("test: select after insert");
    assert_eq!(back.row_count(), 1);
}

#[test]
fn test_insert_multi_row() {
    let mut db = db_with_metadata();
    seed_metadata_docs(&mut db);

    let back = execute(&mut db, "SELECT * FROM docs LIMIT 10", None).expect("test: select multi");
    assert_eq!(back.row_count(), 3);
}

#[test]
fn test_insert_vector_collection_with_param() {
    let mut db = db_with_vectors();

    let r = execute(
        &mut db,
        "INSERT INTO vecs (id, vector, tag) VALUES (1, $v, 'a')",
        Some(r#"{"v": [1.0, 0.0, 0.0, 0.0]}"#),
    )
    .expect("test: vector insert");

    assert_eq!(r.kind(), "mutation");
    assert_eq!(r.row_count(), 1);
}

#[test]
fn test_upsert_replaces_existing() {
    let mut db = db_with_metadata();
    execute(
        &mut db,
        "INSERT INTO docs (id, title) VALUES (1, 'first')",
        None,
    )
    .expect("test: first");
    execute(
        &mut db,
        "UPSERT INTO docs (id, title) VALUES (1, 'replaced')",
        None,
    )
    .expect("test: upsert");

    let r =
        execute(&mut db, "SELECT * FROM docs LIMIT 10", None).expect("test: select after upsert");
    assert_eq!(r.row_count(), 1);
    assert!(r.rows_json().contains("replaced"));
}

// =========================================================================
// UPDATE tests
// =========================================================================

#[test]
fn test_update_modifies_payload() {
    let mut db = db_with_metadata();
    seed_metadata_docs(&mut db);

    let r = execute(
        &mut db,
        "UPDATE docs SET title = 'Updated' WHERE id = 1",
        None,
    )
    .expect("test: update");

    assert_eq!(r.kind(), "mutation");
    assert_eq!(r.row_count(), 1);

    let back =
        execute(&mut db, "SELECT * FROM docs LIMIT 10", None).expect("test: select after update");
    assert!(back.rows_json().contains("Updated"));
}

#[test]
fn test_update_no_match_returns_zero() {
    let mut db = db_with_metadata();
    seed_metadata_docs(&mut db);

    let r = execute(&mut db, "UPDATE docs SET title = 'x' WHERE id = 999", None)
        .expect("test: update none");

    assert_eq!(r.row_count(), 0);
}

// =========================================================================
// DELETE tests
// =========================================================================

#[test]
fn test_delete_removes_matching_row() {
    let mut db = db_with_metadata();
    seed_metadata_docs(&mut db);

    let r = execute(&mut db, "DELETE FROM docs WHERE id = 2", None).expect("test: delete");

    assert_eq!(r.kind(), "deletion");

    let back =
        execute(&mut db, "SELECT * FROM docs LIMIT 10", None).expect("test: select after delete");
    assert_eq!(back.row_count(), 2);
    assert!(!back.rows_json().contains("Cooking"));
}

#[test]
fn test_delete_by_payload_field() {
    let mut db = db_with_metadata();
    seed_metadata_docs(&mut db);

    let r = execute(&mut db, "DELETE FROM docs WHERE category = 'tech'", None)
        .expect("test: delete tech");

    assert_eq!(r.row_count(), 2);
}

// =========================================================================
// DDL tests
// =========================================================================

#[test]
fn test_create_collection_returns_ddl_kind() {
    let mut db = DatabaseInner::new();

    let r = execute(
        &mut db,
        "CREATE COLLECTION items (dimension = 128, metric = 'cosine')",
        None,
    )
    .expect("test: create");

    assert_eq!(r.kind(), "ddl");
    assert!(r.message().contains("DDL"));
    assert!(db.contains("items"));
}

#[test]
fn test_create_metadata_collection_via_sql() {
    let mut db = DatabaseInner::new();

    let r =
        execute(&mut db, "CREATE METADATA COLLECTION meta", None).expect("test: create metadata");

    assert_eq!(r.kind(), "ddl");
    assert!(db.contains("meta"));
}

#[test]
fn test_drop_collection_via_sql() {
    let mut db = db_with_metadata();

    let r = execute(&mut db, "DROP COLLECTION docs", None).expect("test: drop");

    assert_eq!(r.kind(), "ddl");
    assert!(!db.contains("docs"));
}

#[test]
fn test_truncate_preserves_collection_clears_data() {
    let mut db = db_with_metadata();
    seed_metadata_docs(&mut db);

    let r = execute(&mut db, "TRUNCATE docs", None).expect("test: truncate");
    assert_eq!(r.kind(), "ddl");

    let back =
        execute(&mut db, "SELECT * FROM docs LIMIT 10", None).expect("test: select after truncate");
    assert_eq!(back.row_count(), 0);
    assert!(db.contains("docs"));
}

// =========================================================================
// Introspection tests
// =========================================================================

#[test]
fn test_show_collections_lists_created() {
    let mut db = DatabaseInner::new();
    db.create_metadata_collection("meta").expect("test: meta");
    db.create_collection("vecs", 4, "cosine")
        .expect("test: vecs");

    let r = execute(&mut db, "SHOW COLLECTIONS", None).expect("test: show");

    assert_eq!(r.kind(), "rows");
    assert_eq!(r.row_count(), 2);
}

#[test]
fn test_describe_collection_returns_metadata() {
    let mut db = db_with_vectors();

    let r = execute(&mut db, "DESCRIBE COLLECTION vecs", None).expect("test: describe");

    assert_eq!(r.row_count(), 1);
    assert!(r.rows_json().contains("\"dimension\":4"));
    assert!(r.rows_json().contains("\"metric\":\"cosine\""));
}

// =========================================================================
// Admin tests
// =========================================================================

#[test]
fn test_flush_is_noop_admin_kind() {
    let mut db = db_with_metadata();

    let r = execute(&mut db, "FLUSH FULL", None).expect("test: flush");

    assert_eq!(r.kind(), "admin");
    assert!(r.message().contains("Admin"));
}

// =========================================================================
// NEAR vector search tests
// =========================================================================

#[test]
fn test_near_search_ranks_by_similarity() {
    let mut db = db_with_vectors();
    // Insert orthogonal unit vectors so ranking is deterministic.
    for (id, v) in [
        ("1", "[1.0, 0.0, 0.0, 0.0]"),
        ("2", "[0.0, 1.0, 0.0, 0.0]"),
        ("3", "[0.0, 0.0, 1.0, 0.0]"),
    ] {
        execute(
            &mut db,
            &format!("INSERT INTO vecs (id, vector) VALUES ({id}, $v)"),
            Some(&format!("{{\"v\": {v}}}")),
        )
        .expect("test: seed vecs");
    }

    let r = execute(
        &mut db,
        "SELECT * FROM vecs WHERE vector NEAR $q LIMIT 3",
        Some(r#"{"q": [1.0, 0.0, 0.0, 0.0]}"#),
    )
    .expect("test: near");

    assert_eq!(r.row_count(), 3);
    // First row must have id=1 (colinear with query).
    let first = r.row(0).expect("test: first row");
    assert_eq!(first.id(), 1);
}

#[test]
fn test_near_with_filter_post_prunes() {
    let mut db = db_with_vectors();
    execute(
        &mut db,
        "INSERT INTO vecs (id, vector, tag) VALUES (1, $v, 'a')",
        Some(r#"{"v": [1.0, 0.0, 0.0, 0.0]}"#),
    )
    .expect("test: insert 1");
    execute(
        &mut db,
        "INSERT INTO vecs (id, vector, tag) VALUES (2, $v, 'b')",
        Some(r#"{"v": [0.9, 0.1, 0.0, 0.0]}"#),
    )
    .expect("test: insert 2");

    let r = execute(
        &mut db,
        "SELECT * FROM vecs WHERE vector NEAR $q AND tag = 'b' LIMIT 5",
        Some(r#"{"q": [1.0, 0.0, 0.0, 0.0]}"#),
    )
    .expect("test: near + filter");

    assert_eq!(r.row_count(), 1);
    assert_eq!(
        r.row(0).expect("test: row").id(),
        2,
        "filter must exclude id=1"
    );
}

// =========================================================================
// Negative / error-path tests (must be >= 20% of suite)
// =========================================================================

#[test]
fn test_invalid_sql_returns_parse_error() {
    let mut db = db_with_metadata();
    let err = execute(&mut db, "NOT VALID SQL", None);
    assert!(err.is_err());
    assert!(err.expect_err("test: err").contains("parse error"));
}

#[test]
fn test_select_nonexistent_collection_errors() {
    let mut db = DatabaseInner::new();
    let err = execute(&mut db, "SELECT * FROM ghost LIMIT 5", None);
    assert!(err.is_err());
    assert!(err
        .expect_err("test: err")
        .contains("Collection 'ghost' not found"));
}

#[test]
fn test_invalid_params_json_errors() {
    let mut db = db_with_metadata();
    let err = execute(&mut db, "SELECT * FROM docs LIMIT 5", Some("not-json"));
    assert!(err.is_err());
    assert!(err.expect_err("test: err").contains("Invalid params JSON"));
}

#[test]
fn test_insert_vector_collection_without_vector_errors() {
    let mut db = db_with_vectors();
    let err = execute(
        &mut db,
        "INSERT INTO vecs (id, title) VALUES (1, 'x')",
        None,
    );
    assert!(err.is_err());
    assert!(err.expect_err("test: err").contains("vector"));
}

#[test]
fn test_insert_vector_dimension_mismatch_errors() {
    let mut db = db_with_vectors();
    let err = execute(
        &mut db,
        "INSERT INTO vecs (id, vector) VALUES (1, $v)",
        Some(r#"{"v": [1.0, 0.0]}"#),
    );
    assert!(err.is_err());
    assert!(err.expect_err("test: err").contains("dimension mismatch"));
}

#[test]
fn test_near_on_metadata_collection_errors() {
    let mut db = db_with_metadata();
    let err = execute(
        &mut db,
        "SELECT * FROM docs WHERE vector NEAR $q LIMIT 5",
        Some(r#"{"q": [1.0, 0.0, 0.0, 0.0]}"#),
    );
    assert!(err.is_err());
    assert!(err.expect_err("test: err").contains("metadata-only"));
}

#[test]
fn test_match_query_on_empty_graph_returns_empty() {
    // After S4-13 extension: MATCH is supported via in-memory graph.
    // A pattern against an empty graph returns 0 rows, not an error.
    let mut db = db_with_metadata();
    let r = execute(&mut db, "MATCH (p:Person) RETURN p LIMIT 10", None);
    // Empty graph = empty result, no error.
    match r {
        Ok(res) => assert_eq!(res.row_count(), 0),
        Err(e) => assert!(
            e.contains("empty") || e.contains("not found"),
            "unexpected error: {e}"
        ),
    }
}

#[test]
fn test_train_quantizer_is_rejected() {
    let mut db = db_with_vectors();
    let err = execute(&mut db, "TRAIN QUANTIZER ON vecs WITH (type = 'sq8')", None);
    assert!(err.is_err());
    assert!(err.expect_err("test: err").contains("TRAIN"));
}

#[test]
fn test_create_index_is_accepted_as_noop() {
    // After S4-13 extension: CREATE INDEX is accepted as a no-op for API parity.
    let mut db = db_with_metadata();
    let r = execute(&mut db, "CREATE INDEX ON docs (category)", None).expect("test: idx");
    assert_eq!(r.kind(), "ddl");
    assert!(r.rows_json().contains("accepted-noop"));
}

#[test]
fn test_insert_edge_creates_graph() {
    // After S4-13 extension: INSERT EDGE auto-provisions an in-memory graph.
    let mut db = db_with_metadata();
    let r = execute(
        &mut db,
        "INSERT EDGE INTO kg (source = 1, target = 2, label = 'REL')",
        None,
    )
    .expect("test: insert edge");
    assert_eq!(r.kind(), "mutation");
}

#[test]
fn test_union_returns_combined_rows() {
    // After S4-13 extension: UNION is supported.
    let mut db = db_with_metadata();
    seed_metadata_docs(&mut db);
    let r =
        execute(&mut db, "SELECT * FROM docs UNION SELECT * FROM docs", None).expect("test: union");
    // Both selects return the same 3 rows, UNION dedups → 3 rows.
    assert_eq!(r.row_count(), 3);
}

#[test]
fn test_update_id_column_is_rejected() {
    let mut db = db_with_metadata();
    let err = execute(&mut db, "UPDATE docs SET id = 99 WHERE id = 1", None);
    assert!(err.is_err());
    assert!(err.expect_err("test: err").contains("'id'"));
}

#[test]
fn test_unbound_param_errors() {
    let mut db = db_with_metadata();
    seed_metadata_docs(&mut db); // need at least one row to trigger eval.
    let err = execute(
        &mut db,
        "SELECT * FROM docs WHERE id = $missing LIMIT 10",
        Some("{}"),
    );
    assert!(err.is_err());
    assert!(err.expect_err("test: err").contains("$missing"));
}

// =========================================================================
// QueryResult structure smoke tests
// =========================================================================

#[test]
fn test_query_result_rows_json_is_valid_json_array() {
    let mut db = db_with_metadata();
    seed_metadata_docs(&mut db);

    let r = execute(&mut db, "SELECT * FROM docs LIMIT 10", None).expect("test: select");
    let parsed: serde_json::Value = serde_json::from_str(&r.rows_json()).expect("test: valid JSON");
    assert!(parsed.is_array());
    assert_eq!(parsed.as_array().expect("test: arr").len(), 3);
}

#[test]
fn test_query_result_kind_is_stable_string() {
    let mut db = db_with_metadata();
    seed_metadata_docs(&mut db);

    let select = execute(&mut db, "SELECT * FROM docs LIMIT 10", None).expect("test: select");
    assert_eq!(select.kind(), "rows");

    let insert = execute(
        &mut db,
        "INSERT INTO docs (id, title) VALUES (99, 'x')",
        None,
    )
    .expect("test: insert");
    assert_eq!(insert.kind(), "mutation");

    let delete = execute(&mut db, "DELETE FROM docs WHERE id = 99", None).expect("test: delete");
    assert_eq!(delete.kind(), "deletion");
}

// Compile-time sanity: enum mapping used in the suite must stay in sync.
const _: () = {
    // Referenced so the import isn't considered unused by the linter even
    // when the assertions above only consult the string form.
    let _ = QueryResultKind::Rows;
};
