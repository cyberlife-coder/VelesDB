#![cfg(feature = "persistence")]
//! DML reads a graph's rows as SELECT does — every stored node, an embedding,
//! a payload or both — and writes each row back by what it stores: an
//! embedded node through the vector upsert, a payload-only node through the
//! node-payload write. `UPDATE` and a non-id `DELETE` used to select through
//! `Collection::get`, which needs a vector, so on a graph without embeddings
//! both were silent no-ops and on one with embeddings they skipped every
//! payload-only node.

use std::collections::HashMap;

use serde_json::{json, Value};
use tempfile::TempDir;
use velesdb_core::velesql::Parser;
use velesdb_core::{Database, Error};

/// `kge`: 2-d embeddings, payload-only node 1 and embedded node 3, both
/// named `x`. `kg`: no embeddings, nodes 1 (`x`) and 2 (`y`).
fn graphs() -> (TempDir, Database) {
    let dir = TempDir::new().expect("test: tempdir");
    let db = Database::open(dir.path()).expect("test: open");
    for sql in [
        "CREATE GRAPH COLLECTION kge (dimension = 2, metric = 'cosine')",
        "INSERT NODE INTO kge (id = 1, payload = '{\"name\":\"x\"}')",
        "CREATE GRAPH COLLECTION kg",
        "INSERT NODE INTO kg (id = 1, payload = '{\"name\":\"x\"}')",
        "INSERT NODE INTO kg (id = 2, payload = '{\"name\":\"y\"}')",
    ] {
        run(&db, sql, &HashMap::new());
    }
    run(
        &db,
        "INSERT INTO kge (id, vector, name) VALUES (3, $v, 'x')",
        &v(json!([1.0, 0.0])),
    );
    (dir, db)
}

fn v(vector: Value) -> HashMap<String, Value> {
    HashMap::from([("v".to_string(), vector)])
}

fn try_run(
    db: &Database,
    sql: &str,
    params: &HashMap<String, Value>,
) -> Result<Vec<velesdb_core::SearchResult>, Error> {
    let query = Parser::parse(sql).unwrap_or_else(|e| panic!("test: parse {sql}: {e}"));
    db.execute_query(&query, params)
}

fn run(
    db: &Database,
    sql: &str,
    params: &HashMap<String, Value>,
) -> Vec<velesdb_core::SearchResult> {
    try_run(db, sql, params).unwrap_or_else(|e| panic!("test: {sql}: {e}"))
}

fn ids(db: &Database, sql: &str, params: &HashMap<String, Value>) -> Vec<u64> {
    let mut ids: Vec<u64> = run(db, sql, params).iter().map(|r| r.point.id).collect();
    ids.sort_unstable();
    ids
}

fn count(db: &Database, collection: &str) -> Value {
    let query = Parser::parse(&format!("SELECT COUNT(*) FROM {collection}")).expect("test: parse");
    db.execute_aggregate(&query, &HashMap::new())
        .expect("test: aggregate")["count"]
        .clone()
}

fn code(db: &Database, sql: &str, params: &HashMap<String, Value>) -> &'static str {
    try_run(db, sql, params)
        .expect_err("test: statement must be refused")
        .code()
}

#[test]
fn update_writes_payload_only_and_embedded_nodes_alike() {
    let (_dir, db) = graphs();
    let updated = run(
        &db,
        "UPDATE kge SET tag = 'u' WHERE name = 'x'",
        &HashMap::new(),
    );
    assert_eq!(updated.len(), 2, "both nodes named x are rows");
    let none = HashMap::new();
    assert_eq!(
        ids(&db, "SELECT * FROM kge WHERE tag = 'u'", &none),
        vec![1, 3]
    );
    // The embedded node keeps its vector; the payload-only one gains none.
    assert_eq!(
        ids(
            &db,
            "SELECT * FROM kge WHERE vector NEAR $v LIMIT 5",
            &v(json!([1.0, 0.0]))
        ),
        vec![3]
    );
    assert_eq!(count(&db, "kge"), json!(2));
}

#[test]
fn update_by_id_reaches_a_payload_only_node() {
    let (_dir, db) = graphs();
    let updated = run(
        &db,
        "UPDATE kge SET tag = 'u' WHERE id = 1",
        &HashMap::new(),
    );
    assert_eq!(updated.len(), 1);
    assert_eq!(
        ids(&db, "SELECT * FROM kge WHERE tag = 'u'", &HashMap::new()),
        vec![1]
    );
}

#[test]
fn dml_on_a_graph_without_embeddings_is_no_longer_a_no_op() {
    let (_dir, db) = graphs();
    let none = HashMap::new();
    let updated = run(&db, "UPDATE kg SET tag = 'u' WHERE name = 'x'", &none);
    assert_eq!(updated.len(), 1);
    assert_eq!(ids(&db, "SELECT * FROM kg WHERE tag = 'u'", &none), vec![1]);
    run(&db, "DELETE FROM kg WHERE name = 'x'", &none);
    assert_eq!(ids(&db, "SELECT * FROM kg", &none), vec![2]);
    assert_eq!(count(&db, "kg"), json!(1));
}

#[test]
fn a_non_id_delete_removes_payload_only_nodes_too() {
    let (_dir, db) = graphs();
    let none = HashMap::new();
    run(&db, "DELETE FROM kge WHERE name = 'x'", &none);
    assert_eq!(ids(&db, "SELECT * FROM kge", &none), Vec::<u64>::new());
    assert_eq!(count(&db, "kge"), json!(0));
}

/// `SET vector` on a payload-only node of a graph with embeddings attaches
/// one — `GraphCollection::upsert_node` with a vector does the same — and
/// the upsert checks its dimension. A graph without embeddings stores no
/// vector, so the assignment is refused there, as on a metadata collection.
#[test]
fn set_vector_attaches_an_embedding_where_the_graph_stores_them() {
    let (_dir, db) = graphs();
    let updated = run(
        &db,
        "UPDATE kge SET vector = $v WHERE id = 1",
        &v(json!([0.0, 1.0])),
    );
    assert_eq!(updated.len(), 1);
    assert_eq!(
        ids(
            &db,
            "SELECT * FROM kge WHERE vector NEAR $v LIMIT 5",
            &v(json!([0.0, 1.0]))
        ),
        vec![1, 3]
    );
    assert_eq!(
        code(
            &db,
            "UPDATE kge SET vector = $v WHERE id = 1",
            &v(json!([1.0, 0.0, 0.0]))
        ),
        "VELES-004"
    );
    assert_eq!(
        code(
            &db,
            "UPDATE kg SET vector = $v WHERE id = 1",
            &v(json!([1.0, 0.0]))
        ),
        "VELES-014"
    );
}

/// A vector written to a collection that stores none is one refusal,
/// `VELES-014`, whichever surface writes it — DML or the collection API —
/// and whichever kind of collection stores no vectors: a metadata
/// collection, a graph without embeddings. DML refused it with `VELES-010`
/// and the API on a graph with `VELES-004` "expected 0".
#[test]
fn a_vector_written_where_no_vector_is_stored_is_refused_with_veles_014() {
    let (_dir, db) = graphs();
    run(&db, "CREATE METADATA COLLECTION m", &HashMap::new());
    run(
        &db,
        "INSERT INTO m (id, k) VALUES (1, 'x')",
        &HashMap::new(),
    );
    for sql in [
        "INSERT INTO m (id, vector) VALUES (2, $v)",
        "UPDATE m SET vector = $v WHERE id = 1",
        "INSERT INTO kg (id, vector) VALUES (3, $v)",
        "UPDATE kg SET vector = $v WHERE id = 1",
    ] {
        assert_eq!(code(&db, sql, &v(json!([1.0, 0.0]))), "VELES-014", "{sql}");
    }
    let point = || velesdb_core::Point::new(4, vec![1.0, 0.0], None);
    for name in ["m", "kg"] {
        let refused = db
            .get_any_collection(name)
            .expect("test: collection")
            .upsert(vec![point()]);
        assert_eq!(
            refused.expect_err("test: refused").code(),
            "VELES-014",
            "{name}"
        );
    }
}

/// INSERT writes a row as the collection stores it: a graph's node may
/// carry no embedding (the row `INSERT NODE` writes), so a row without a
/// `vector` is a payload-only node; a graph without embeddings refuses one.
#[test]
fn insert_without_a_vector_writes_a_payload_only_node() {
    let (_dir, db) = graphs();
    let none = HashMap::new();
    for graph in ["kg", "kge"] {
        let inserted = run(
            &db,
            &format!("INSERT INTO {graph} (id, name) VALUES (5, 'w')"),
            &none,
        );
        assert_eq!(inserted.len(), 1, "{graph}");
        assert_eq!(
            ids(
                &db,
                &format!("SELECT * FROM {graph} WHERE name = 'w'"),
                &none
            ),
            vec![5],
            "{graph}"
        );
    }
    assert_eq!(count(&db, "kg"), json!(3));
    assert_eq!(
        code(
            &db,
            "INSERT INTO kg (id, vector, name) VALUES (6, $v, 'w')",
            &v(json!([1.0, 0.0]))
        ),
        "VELES-014"
    );
    // A vector collection still requires one: a row without one is refused
    // as the collection API refuses it.
    run(&db, "CREATE COLLECTION docs (dimension = 2)", &none);
    assert_eq!(
        code(&db, "INSERT INTO docs (id, name) VALUES (1, 'a')", &none),
        "VELES-016"
    );
}

/// INSERT, UPSERT and UPDATE return the rows they leave stored, as a SELECT
/// reads them back: a row written without a vector keeps the embedding the
/// node has. They used to return the rows as the statement wrote them — an
/// empty `vector` where the node kept `[0, 1]`.
#[test]
fn dml_returns_the_rows_it_leaves_stored() {
    let (_dir, db) = graphs();
    let none = HashMap::new();
    let stored = |results: &[velesdb_core::SearchResult]| -> Vec<(u64, Vec<f32>, Option<Value>)> {
        results
            .iter()
            .map(|r| (r.point.id, r.point.vector.clone(), r.point.payload.clone()))
            .collect()
    };
    run(
        &db,
        "INSERT INTO kge (id, vector) VALUES (2, $v)",
        &v(json!([0.0, 1.0])),
    );
    let upserted = run(&db, "UPSERT INTO kge (id, a) VALUES (2, 7)", &none);
    let selected = run(&db, "SELECT * FROM kge WHERE id = 2", &none);
    assert_eq!(
        stored(&upserted),
        vec![(2, vec![0.0, 1.0], Some(json!({"a": 7})))]
    );
    assert_eq!(stored(&upserted), stored(&selected), "as SELECT reads it");

    let updated = run(&db, "UPDATE kge SET b = 8 WHERE id = 2", &none);
    assert_eq!(
        stored(&updated),
        vec![(2, vec![0.0, 1.0], Some(json!({"a": 7, "b": 8})))]
    );
}

/// A node written after `CREATE INDEX` is in the index: the node-payload
/// write used to maintain the label and range indexes but not the secondary
/// ones, so an indexed SELECT missed it while `COUNT(*)` counted it.
#[test]
fn node_writes_maintain_secondary_indexes() {
    let (_dir, db) = graphs();
    let none = HashMap::new();
    run(&db, "CREATE INDEX ON kg (name)", &none);
    run(
        &db,
        "INSERT NODE INTO kg (id = 3, payload = '{\"name\":\"x\"}')",
        &none,
    );
    assert_eq!(
        ids(&db, "SELECT * FROM kg WHERE name = 'x'", &none),
        vec![1, 3]
    );
    run(&db, "UPDATE kg SET name = 'z' WHERE id = 1", &none);
    assert_eq!(
        ids(&db, "SELECT * FROM kg WHERE name = 'x'", &none),
        vec![3]
    );
    assert_eq!(
        ids(&db, "SELECT * FROM kg WHERE name = 'z'", &none),
        vec![1]
    );
}

/// A DML statement is refused for its shape before its parameters are
/// bound — the order a SELECT follows, validated before it executes — so a
/// vector predicate in the WHERE answers VELES-010 even when its `$name` is
/// unbound, rather than VELES-038.
#[test]
fn a_dml_where_is_refused_for_its_shape_before_binding() {
    let (_dir, db) = graphs();
    let none = HashMap::new();
    assert_eq!(
        code(
            &db,
            "DELETE FROM kge WHERE similarity(vector, $x) > 0.5",
            &none
        ),
        "VELES-010"
    );
    assert_eq!(
        code(
            &db,
            "UPDATE kge SET vector = $v WHERE similarity(vector, $x) > 0.5",
            &none
        ),
        "VELES-010"
    );
    // A metadata WHERE with an unbound parameter is a parameter error.
    assert_eq!(
        code(&db, "DELETE FROM kge WHERE name = $x", &none),
        "VELES-038"
    );
    assert_eq!(
        code(&db, "UPDATE kge SET tag = 'u' WHERE name = $x", &none),
        "VELES-038"
    );
}

/// A full-text `MATCH` is no metadata predicate: the filter conversion
/// drops it, so under an `AND` it used to widen the rows a DML statement
/// touched — every node named x deleted, whatever its text — instead of
/// being refused as the other ranking predicates are.
#[test]
fn a_text_match_in_a_dml_where_is_refused_not_dropped() {
    let (_dir, db) = graphs();
    let none = HashMap::new();
    assert_eq!(
        code(
            &db,
            "DELETE FROM kge WHERE name = 'x' AND body MATCH 'nothing'",
            &none
        ),
        "VELES-010"
    );
    assert_eq!(
        code(
            &db,
            "UPDATE kge SET tag = 'u' WHERE name = 'x' AND body MATCH 'nothing'",
            &none
        ),
        "VELES-010"
    );
    assert_eq!(count(&db, "kge"), json!(2), "nothing deleted");
}

/// Full-text `MATCH` reads a graph's payload-only nodes, in a graph with
/// embeddings and one without: the node-payload write (`INSERT NODE`, an
/// `INSERT` without a vector) never reached BM25, and the text search
/// hydrated only rows with a vector — on a graph without embeddings every
/// `MATCH` answered no row.
#[test]
fn full_text_match_reads_payload_only_nodes() {
    let (_dir, db) = graphs();
    let none = HashMap::new();
    for graph in ["kg", "kge"] {
        run(
            &db,
            &format!("INSERT NODE INTO {graph} (id = 7, payload = '{{\"body\":\"hello world\"}}')"),
            &none,
        );
        run(
            &db,
            &format!("INSERT INTO {graph} (id, body) VALUES (8, 'hello there')"),
            &none,
        );
        assert_eq!(
            ids(
                &db,
                &format!("SELECT * FROM {graph} WHERE body MATCH 'hello' LIMIT 10"),
                &none
            ),
            vec![7, 8],
            "{graph}"
        );
    }
}

/// A payload write over an embedded node replaces its terms: the text it
/// no longer carries stops matching, its new text matches, and the node
/// keeps its embedding.
#[test]
fn a_payload_write_over_an_embedded_node_replaces_its_terms() {
    let (_dir, db) = graphs();
    let none = HashMap::new();
    run(
        &db,
        "INSERT INTO kge (id, vector, body) VALUES (5, $v, 'hello again')",
        &v(json!([1.0, 0.0])),
    );
    run(
        &db,
        "INSERT INTO kge (id, body) VALUES (5, 'bye now')",
        &none,
    );
    let matched = |text: &str| {
        ids(
            &db,
            &format!("SELECT * FROM kge WHERE body MATCH '{text}' LIMIT 10"),
            &none,
        )
    };
    assert_eq!(matched("again"), Vec::<u64>::new());
    assert_eq!(matched("bye"), vec![5]);
    run(
        &db,
        "INSERT NODE INTO kge (id = 5, payload = '{\"body\":\"later\"}')",
        &none,
    );
    assert_eq!(matched("bye"), Vec::<u64>::new());
    assert_eq!(matched("later"), vec![5]);
    assert_eq!(
        ids(
            &db,
            "SELECT * FROM kge WHERE vector NEAR $v LIMIT 5",
            &v(json!([1.0, 0.0]))
        ),
        vec![3, 5]
    );
}

/// A `vector` the statement binds is a vector, never "no vector": an empty
/// one is refused on a graph with embeddings as a vector of the wrong
/// dimension (VELES-004) — it used to write a payload-only node on INSERT
/// and leave the old embedding in place on UPDATE. A vector collection
/// refuses it too, as the row without a vector it would be (VELES-016). A
/// graph row without the column still writes a node without an embedding.
#[test]
fn an_empty_vector_is_refused_as_a_vector_collection_refuses_it() {
    let (_dir, db) = graphs();
    let empty = v(json!([]));
    run(
        &db,
        "CREATE COLLECTION docs (dimension = 2)",
        &HashMap::new(),
    );
    for sql in [
        "INSERT INTO kge (id, vector, name) VALUES (8, $v, 'e')",
        "UPDATE kge SET vector = $v WHERE id = 3",
        "UPDATE kge SET vector = $v WHERE id = 1",
    ] {
        assert_eq!(code(&db, sql, &empty), "VELES-004", "{sql}");
    }
    assert_eq!(
        code(
            &db,
            "INSERT INTO docs (id, vector, name) VALUES (8, $v, 'e')",
            &empty
        ),
        "VELES-016"
    );
    assert_eq!(count(&db, "kge"), json!(2), "nothing written");
    assert_eq!(
        ids(
            &db,
            "SELECT * FROM kge WHERE vector NEAR $v LIMIT 5",
            &v(json!([1.0, 0.0]))
        ),
        vec![3],
        "node 3 keeps its embedding"
    );
}
