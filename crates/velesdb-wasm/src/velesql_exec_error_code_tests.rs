//! The `VELES-XXX` code a VelesQL executor error carries to JS.
//!
//! `executeQuery` and `executeMatch` fail with an [`ExecError`]: an error
//! core defines stays a `velesdb_core::Error` ([`ExecError::Core`]) from
//! where it is raised to the FFI boundary, which reads its code from the
//! variant and throws a JS `Error` whose `code` the TypeScript SDK maps to
//! its typed class. These scenarios pin that a `$name` the executor cannot
//! bind is core's VELES-038 with core's message on every path that binds
//! one, that a missing collection is core's VELES-002 on every statement
//! that names one, and that the executor's own refusals carry no code.

use serde_json::json;
use velesdb_core::Error as CoreError;

use crate::database::DatabaseInner;
use crate::exec_error::ExecError;
use crate::velesql_exec::execute;
use crate::velesql_match_core::execute_match_rows;

/// `vecs` (cosine, 2-d): person 100 (`[0, 1]`) WROTE doc 1 (`[1, 0]`).
fn db() -> DatabaseInner {
    let mut db = DatabaseInner::new();
    db.create_collection("vecs", 2, "cosine")
        .expect("test: create");
    let store = db.get_shared_store("vecs").expect("test: store");
    for (id, vector, label) in [(100u64, [0.0, 1.0], "Person"), (1, [1.0, 0.0], "Doc")] {
        crate::store_insert::insert_with_payload(
            &mut store.borrow_mut(),
            id,
            &vector,
            Some(json!({"_labels": [label], "cat": "tech"})),
        );
    }
    execute(
        &mut db,
        "INSERT EDGE INTO vecs (id = 7, source = 100, target = 1, label = 'WROTE')",
        None,
    )
    .expect("test: edge");
    db
}

/// The error `executeQuery` raises for `sql`.
fn query_refusal(sql: &str, params: Option<&str>) -> ExecError {
    match execute(&mut db(), sql, params) {
        Ok(_) => panic!("test: `{sql}` must be refused"),
        Err(e) => e,
    }
}

/// The core error `err` carries, which it must.
fn core_error(err: ExecError) -> CoreError {
    match err {
        ExecError::Core(core) => core,
        ExecError::Refused(message) => panic!("test: expected a core error, got {message:?}"),
    }
}

/// Asserts `err` is core's VELES-038 for `$name`, word for word.
fn assert_unbound(err: ExecError, name: &str) {
    let core = core_error(err);
    assert_eq!(core.code(), "VELES-038", "{core}");
    let unbound = CoreError::QueryParameter {
        name: name.to_string(),
        reason: "not bound".to_string(),
    };
    assert_eq!(core.to_string(), unbound.to_string());
}

/// Asserts `err` is core's VELES-038 naming `$name`.
fn assert_parameter_error(err: ExecError, name: &str) {
    let core = core_error(err);
    assert!(
        matches!(&core, CoreError::QueryParameter { name: n, .. } if n == name),
        "expected a parameter error naming ${name}, got {core}"
    );
}

#[test]
fn an_unbound_where_parameter_is_veles_038() {
    let err = query_refusal("SELECT * FROM vecs WHERE cat = $c", None);
    assert_unbound(err, "c");
}

#[test]
fn an_unbound_near_vector_is_veles_038() {
    let err = query_refusal("SELECT * FROM vecs WHERE vector NEAR $q", None);
    assert_unbound(err, "q");
}

#[test]
fn a_near_vector_bound_to_a_string_is_veles_038() {
    let err = query_refusal(
        "SELECT * FROM vecs WHERE vector NEAR $q",
        Some(r#"{"q": "not a vector"}"#),
    );
    assert_parameter_error(err, "q");
}

#[test]
fn an_unbound_similarity_vector_is_veles_038() {
    let err = query_refusal(
        "SELECT * FROM vecs WHERE similarity(vector, $q) > 0.5",
        None,
    );
    assert_unbound(err, "q");
}

#[test]
fn an_unbound_insert_value_is_veles_038() {
    let err = query_refusal(
        "INSERT INTO vecs (id, vector, cat) VALUES (2, $v, $c)",
        Some(r#"{"v": [1.0, 0.0]}"#),
    );
    assert_unbound(err, "c");
}

#[test]
fn an_unbound_insert_vector_is_veles_038() {
    let err = query_refusal(
        "INSERT INTO vecs (id, vector, cat) VALUES (2, $v, 'x')",
        None,
    );
    assert_unbound(err, "v");
}

/// An `INSERT` or `UPDATE` vector `$v` bound to a value that is no vector
/// is core's parameter error, as on core's DML path: the executor narrowed
/// it itself and refused it with a message of its own.
#[test]
fn a_dml_vector_bound_to_no_vector_is_veles_038() {
    for sql in [
        "INSERT INTO vecs (id, vector, cat) VALUES (2, $v, 'x')",
        "UPDATE vecs SET vector = $v WHERE id = 1",
    ] {
        for bound in [r#"{"v": "x"}"#, r#"{"v": [1.0, "x"]}"#] {
            assert_parameter_error(query_refusal(sql, Some(bound)), "v");
        }
    }
}

/// A DML vector of another dimension is core's VELES-004, on INSERT as on
/// UPDATE.
#[test]
fn a_dml_vector_of_another_dimension_is_veles_004() {
    for sql in [
        "INSERT INTO vecs (id, vector, cat) VALUES (2, $v, 'x')",
        "UPDATE vecs SET vector = $v WHERE id = 1",
    ] {
        let core = core_error(query_refusal(sql, Some(r#"{"v": [1.0, 0.0, 0.0]}"#)));
        assert!(
            matches!(
                core,
                CoreError::DimensionMismatch {
                    expected: 2,
                    actual: 3
                }
            ),
            "{sql}: {core}"
        );
    }
}

/// A DML WHERE is refused for its shape before any `$name` is bound, by
/// core's rule (`velesql::check_dml_where`): a vector, graph or full-text
/// predicate is core's VELES-010, word for word, even when a `$name` of the
/// statement is unbound. WASM bound the `SET` list first (VELES-038 for an
/// unbound `$nope`) and refused the predicate with an uncoded string.
#[test]
fn a_dml_where_is_refused_for_its_shape_before_binding() {
    for (sql, statement) in [
        (
            "UPDATE vecs SET cat = $nope WHERE similarity(vector, $q) > 0.1",
            "UPDATE",
        ),
        (
            "UPDATE vecs SET cat = 'x' WHERE cat = 'tech' AND body MATCH 'x'",
            "UPDATE",
        ),
        ("DELETE FROM vecs WHERE vector NEAR $q", "DELETE"),
        (
            "DELETE FROM vecs WHERE body MATCH 'x' AND cat = $nope",
            "DELETE",
        ),
    ] {
        let core = core_error(query_refusal(sql, None));
        let expected = CoreError::Query(format!(
            "{statement} WHERE supports metadata predicates only (no similarity/NEAR/MATCH)"
        ));
        assert_eq!(core.to_string(), expected.to_string(), "{sql}");
    }
}

#[test]
fn a_having_parameter_unbound_or_composite_is_veles_038() {
    let sql = "SELECT cat, COUNT(*) FROM vecs GROUP BY cat HAVING COUNT(*) > $n";
    assert_unbound(query_refusal(sql, None), "n");
    assert_parameter_error(query_refusal(sql, Some(r#"{"n": [1]}"#)), "n");
}

#[test]
fn an_unbound_match_similarity_vector_is_veles_038() {
    let err = execute_match_rows(
        &mut db(),
        "MATCH (a:Person)-[:WROTE]->(d:Doc) WHERE similarity(d.vector, $v) > 0.5 RETURN d.cat",
        Some(r#"{"_collection": "vecs"}"#),
        None,
    )
    .expect_err("test: unbound $v");
    assert_unbound(err, "v");
}

/// Every statement that names a missing collection fails with core's
/// `CollectionNotFound` — `DROP COLLECTION`, `FLUSH` and the edge reads,
/// which raised an uncoded message of their own, included. (`INSERT EDGE`
/// and `INSERT NODE` provision a graph store of their own in WASM.)
#[test]
fn a_missing_collection_is_veles_002_on_every_statement() {
    for sql in [
        "SELECT * FROM ghost",
        "DROP COLLECTION ghost",
        "FLUSH ghost",
        "TRUNCATE COLLECTION ghost",
        "INSERT INTO ghost (id, cat) VALUES (1, 'x')",
        "SELECT EDGES FROM ghost",
        "DELETE EDGE 1 FROM ghost",
    ] {
        let core = core_error(query_refusal(sql, None));
        assert_eq!(
            core.to_string(),
            CoreError::CollectionNotFound("ghost".to_string()).to_string(),
            "{sql}"
        );
    }
}

/// `DROP COLLECTION IF EXISTS` on a missing collection is no error.
#[test]
fn drop_if_exists_of_a_missing_collection_succeeds() {
    execute(&mut db(), "DROP COLLECTION IF EXISTS ghost", None).expect("test: no error");
}

/// A collection created twice is core's VELES-001.
#[test]
fn a_collection_created_twice_is_veles_001() {
    let core = core_error(query_refusal(
        "CREATE COLLECTION vecs (dimension = 2, metric = 'cosine')",
        None,
    ));
    assert_eq!(core.code(), "VELES-001", "{core}");
}

/// A parse failure is core's query error.
#[test]
fn a_parse_failure_is_veles_010() {
    let core = core_error(query_refusal("SELEC * FROM vecs", None));
    assert_eq!(core.code(), "VELES-010", "{core}");
}

#[test]
fn an_executor_refusal_of_its_own_carries_no_code() {
    let err = execute_match_rows(&mut db(), "SELECT * FROM vecs", None, None)
        .expect_err("test: not a MATCH");
    assert!(matches!(err, ExecError::Refused(_)), "{err}");
    assert_eq!(err.code(), None);
}

// --- Core's coded refusals, word for word -------------------------------
//
// Each case below is a rule core enforces with a coded variant; the WASM
// executor raises the same variant with the same message (pinned against
// core's own output for the same statement).

/// [`db`] plus `meta`, a metadata-only collection holding row 1.
fn db_with_meta() -> DatabaseInner {
    let mut db = db();
    db.create_metadata_collection("meta")
        .expect("test: create meta");
    execute(&mut db, "INSERT INTO meta (id, cat) VALUES (1, 'x')", None).expect("test: row");
    db
}

/// The core error `executeQuery` raises for `sql` on [`db_with_meta`].
fn core_refusal(sql: &str, params: &str) -> CoreError {
    match execute(&mut db_with_meta(), sql, Some(params)) {
        Ok(_) => panic!("test: `{sql}` must be refused"),
        Err(e) => core_error(e),
    }
}

/// Asserts each `(sql, params)` fails with exactly `expected`.
fn assert_core_refusals(cases: &[(&str, &str)], expected: &str) {
    for (sql, params) in cases {
        assert_eq!(core_refusal(sql, params).to_string(), expected, "{sql}");
    }
}

#[test]
fn a_vector_search_on_a_metadata_collection_is_veles_015() {
    assert_core_refusals(
        &[
            (
                "SELECT * FROM meta WHERE vector NEAR $v LIMIT 5",
                r#"{"v": [1.0, 0.0]}"#,
            ),
            (
                "SELECT * FROM meta WHERE similarity(vector, $v) > 0.5 LIMIT 5",
                r#"{"v": [1.0, 0.0]}"#,
            ),
            // Whatever the operator, the leaf's place or the scan that runs,
            // as core checks every query vector before any row
            // (`QueryVectorTarget`).
            (
                "SELECT * FROM meta WHERE similarity(vector, $v) < 0.5 LIMIT 5",
                r#"{"v": [1.0, 0.0]}"#,
            ),
            (
                "SELECT * FROM meta WHERE NOT similarity(vector, $v) > 0.5 LIMIT 5",
                r#"{"v": [1.0, 0.0]}"#,
            ),
            (
                "SELECT * FROM meta WHERE similarity(vector, $v) > 0.5 OR cat = 'x' LIMIT 5",
                r#"{"v": [1.0, 0.0]}"#,
            ),
            (
                "SELECT * FROM meta ORDER BY similarity(vector, $v) DESC LIMIT 5",
                r#"{"v": [1.0, 0.0]}"#,
            ),
            (
                "SELECT * FROM meta WHERE vector NEAR_FUSED [$v, $v] LIMIT 5",
                r#"{"v": [1.0, 0.0]}"#,
            ),
            (
                "MATCH (a) WHERE similarity(a.vector, $v) > 0.5 RETURN a",
                r#"{"v": [1.0, 0.0], "_collection": "meta"}"#,
            ),
            (
                "MATCH (a) RETURN a ORDER BY similarity(a.vector, $v) DESC",
                r#"{"v": [1.0, 0.0], "_collection": "meta"}"#,
            ),
        ],
        "[VELES-015] Vector search not supported on 'meta': it stores no vectors. \
         Use query() instead.",
    );
}

#[test]
fn a_query_vector_of_another_dimension_is_veles_004() {
    assert_core_refusals(
        &[
            (
                "SELECT * FROM vecs WHERE vector NEAR $v LIMIT 5",
                r#"{"v": [1.0, 0.0, 0.0]}"#,
            ),
            (
                "SELECT * FROM vecs WHERE similarity(vector, $v) > 0.1 LIMIT 5",
                r#"{"v": [1.0, 0.0, 0.0]}"#,
            ),
            (
                "SELECT * FROM vecs WHERE vector NEAR_FUSED [$v, $v] LIMIT 5",
                r#"{"v": [1.0, 0.0, 0.0]}"#,
            ),
        ],
        "[VELES-004] Vector dimension mismatch: expected 2, got 3",
    );
}

/// More fused vectors than core's `multi_query_search` fuses is its
/// configuration error (a metadata collection refuses any fused vector with
/// VELES-015, pinned with the other query vectors above).
#[test]
fn near_fused_refusals_are_core_s_multi_query_ones() {
    let eleven = ["$v"; velesdb_core::fusion::MAX_FUSED_VECTORS + 1].join(", ");
    assert_core_refusals(
        &[(
            &format!("SELECT * FROM vecs WHERE vector NEAR_FUSED [{eleven}] LIMIT 5"),
            r#"{"v": [1.0, 0.0]}"#,
        )],
        &format!(
            "[VELES-009] Configuration error: multi_query_search supports at most {} vectors, \
             got {}",
            velesdb_core::fusion::MAX_FUSED_VECTORS,
            velesdb_core::fusion::MAX_FUSED_VECTORS + 1
        ),
    );
}

/// An `executeMatch` query vector the collection cannot score: another
/// dimension (VELES-004), or no vectors at all (VELES-015), as core's REST
/// `/match` refuses it.
#[test]
fn match_query_vector_refusals_are_core_s() {
    for (collection, expected) in [
        (
            "vecs",
            "[VELES-004] Vector dimension mismatch: expected 2, got 3",
        ),
        (
            "meta",
            "[VELES-015] Vector search not supported on 'meta': it stores no vectors. \
             Use query() instead.",
        ),
    ] {
        let scoring =
            crate::velesql_match_score::MatchScoring::from_request(Some(vec![1.0; 3]), None)
                .expect("test: valid scoring");
        let params = json!({ "_collection": collection }).to_string();
        let err = execute_match_rows(
            &mut db_with_meta(),
            "MATCH (a) RETURN a LIMIT 5",
            Some(&params),
            scoring,
        )
        .expect_err("test: refused");
        assert_eq!(core_error(err).to_string(), expected, "{collection}");
    }
}

/// A MATCH naming a missing collection is core's VELES-002; a `_collection`
/// that is no collection name is core's VELES-038 naming the key.
#[test]
fn a_match_collection_param_is_resolved_by_core_s_rule() {
    let run = |params: serde_json::Value| {
        execute_match_rows(
            &mut db(),
            "MATCH (a) RETURN a LIMIT 5",
            Some(&params.to_string()),
            None,
        )
        .expect_err("test: refused")
    };
    assert_eq!(
        core_error(run(json!({ "_collection": "ghost" }))).to_string(),
        CoreError::CollectionNotFound("ghost".to_string()).to_string()
    );
    for bad in [json!(""), json!(7)] {
        let core = core_error(run(json!({ "_collection": bad })));
        assert!(
            matches!(&core, CoreError::QueryParameter { name, .. } if name == "_collection"),
            "{core}"
        );
    }
}

/// A vector written into the metadata-only collection is refused as core
/// refuses it on every write surface: `VectorNotAllowed`, VELES-014.
#[test]
fn a_vector_written_into_a_metadata_collection_is_veles_014() {
    assert_core_refusals(
        &[
            (
                "INSERT INTO meta (id, vector) VALUES (5, $v)",
                r#"{"v": [1.0]}"#,
            ),
            (
                "UPDATE meta SET vector = $v WHERE id = 1",
                r#"{"v": [1.0]}"#,
            ),
        ],
        &CoreError::VectorNotAllowed("meta".to_string()).to_string(),
    );
}

#[test]
fn insert_refusals_are_core_s_query_errors() {
    for (sql, params, message) in [
        (
            "INSERT INTO vecs (cat, vector) VALUES ('x', $v)",
            r#"{"v": [1.0, 0.0]}"#,
            "INSERT requires integer 'id' column",
        ),
        (
            "INSERT INTO vecs (id, vector) VALUES (5, NULL)",
            "{}",
            "'vector' must be an array of numbers",
        ),
        (
            "INSERT INTO vecs (id, vector) VALUES (5, 'x')",
            "{}",
            "'vector' must be an array of numbers",
        ),
        (
            "INSERT INTO meta (id, cat) VALUES ('a', 'x')",
            "{}",
            "id must be an integer",
        ),
        (
            "INSERT INTO meta (id, cat) VALUES ($i, 'x')",
            r#"{"i": -1}"#,
            "id must be >= 0",
        ),
    ] {
        assert_eq!(
            core_refusal(sql, params).to_string(),
            CoreError::Query(message.to_string()).to_string(),
            "{sql}"
        );
    }
}

/// A row without a vector on a vector collection is refused as core refuses
/// it, `VELES-016`: without the `vector` column or with an empty one, by
/// INSERT and by UPDATE.
#[test]
fn a_row_without_a_vector_on_a_vector_collection_is_veles_016() {
    assert_core_refusals(
        &[
            ("INSERT INTO vecs (id, cat) VALUES (5, 'x')", "{}"),
            (
                "INSERT INTO vecs (id, vector) VALUES (5, $v)",
                r#"{"v": []}"#,
            ),
            ("UPDATE vecs SET vector = $v WHERE id = 1", r#"{"v": []}"#),
        ],
        &CoreError::VectorRequired("vecs".to_string()).to_string(),
    );
}

#[test]
fn update_refusals_are_core_s_query_errors() {
    for (sql, params, message) in [
        (
            "UPDATE vecs SET id = 3",
            "{}",
            "UPDATE cannot modify primary key column 'id'",
        ),
        (
            "UPDATE vecs SET vector = 'x' WHERE id = 1",
            "{}",
            "'vector' must be an array of numbers",
        ),
    ] {
        assert_eq!(
            core_refusal(sql, params).to_string(),
            CoreError::Query(message.to_string()).to_string(),
            "{sql}"
        );
    }
}

/// As in core, an UPDATE binds its `SET` values before it reads a row, and
/// refuses a vector on a metadata collection per matched row: an unbound
/// `$w` is VELES-038 there too, and a WHERE matching no row updates none.
#[test]
fn an_update_binds_first_and_refuses_a_metadata_vector_per_row() {
    assert_unbound(
        query_refusal_on_meta("UPDATE meta SET vector = $w", "{}"),
        "w",
    );
    for sql in [
        "UPDATE meta SET vector = $v WHERE id = 999",
        "UPDATE vecs SET vector = $v WHERE id = 999",
    ] {
        execute(&mut db_with_meta(), sql, Some(r#"{"v": [1.0, 0.0]}"#))
            .unwrap_or_else(|e| panic!("test: `{sql}` updates no row: {e}"));
    }
}

/// The error `executeQuery` raises for `sql` on [`db_with_meta`].
fn query_refusal_on_meta(sql: &str, params: &str) -> ExecError {
    match execute(&mut db_with_meta(), sql, Some(params)) {
        Ok(_) => panic!("test: `{sql}` must be refused"),
        Err(e) => e,
    }
}

/// `CREATE COLLECTION` resolves its storage mode by core's DDL rule
/// (`velesql::resolve_storage_mode`). (An unknown metric is refused by the
/// parser before either engine runs.)
#[test]
fn an_unknown_ddl_storage_mode_is_core_s_query_error() {
    let mode = core_error(query_refusal(
        "CREATE COLLECTION c (dimension = 2, metric = 'cosine') WITH (storage = 'bogus')",
        None,
    ));
    assert_eq!(
        mode.to_string(),
        velesdb_core::velesql::resolve_storage_mode(Some("bogus"))
            .expect_err("test: core refuses it")
            .to_string()
    );
}

/// A text `MATCH` in a SELECT WHERE is refused, never dropped from it: the
/// WASM executor has no BM25 index, so a `MATCH` core runs — AND-ed, fused
/// with a `NEAR`, under `OR` or `NOT` — is its own refusal (no code), and one
/// core refuses too, a second ranking `MATCH`, is refused first by the
/// validation core runs (`velesql::validate_vector_predicates`), with core's
/// error.
#[test]
fn a_text_match_in_a_select_is_refused_never_dropped() {
    for sql in [
        "SELECT * FROM vecs WHERE body MATCH 'x' AND cat = 'tech' LIMIT 2",
        "SELECT * FROM vecs WHERE vector NEAR $q AND body MATCH 'x' LIMIT 2",
        "SELECT * FROM vecs WHERE body MATCH 'x' OR cat = 'tech' LIMIT 2",
        "SELECT * FROM vecs WHERE NOT (body MATCH 'x') LIMIT 2",
    ] {
        let err = query_refusal(sql, Some(r#"{"q": [1.0, 0.0]}"#));
        assert!(matches!(err, ExecError::Refused(_)), "{sql}: {err}");
        assert!(err.to_string().contains("MATCH"), "{sql}: {err}");
    }
    let sql = "SELECT * FROM vecs WHERE body MATCH 'x' AND body MATCH 'y' LIMIT 2";
    let core = core_error(query_refusal(sql, None));
    assert_eq!(core.code(), "VELES-010", "{sql}: {core}");
    assert!(core.to_string().contains("MATCH"), "{sql}: {core}");
}
