//! The row an `INSERT` writes and the `SET` list an `UPDATE` applies, as
//! every engine binds them.

use super::*;

use serde_json::json;

use crate::velesql::{Parser, Query};

type Params = HashMap<String, serde_json::Value>;

/// A collection `t` storing 2-d vectors.
const STORES_2D: QueryVectorTarget<'static> = QueryVectorTarget {
    collection: "t",
    dimension: Some(2),
};
/// A collection `t` storing no vector.
const STORES_NONE: QueryVectorTarget<'static> = QueryVectorTarget {
    collection: "t",
    dimension: None,
};
/// The column of a vector collection of 2-d vectors.
const VECTORS: VectorColumn<'static> = STORES_2D.vector_column(false);
/// The column of a graph with 2-d embeddings.
const EMBEDDINGS: VectorColumn<'static> = STORES_2D.vector_column(true);
/// The column of a metadata collection.
const NO_VECTORS: VectorColumn<'static> = STORES_NONE.vector_column(false);

fn parse(sql: &str) -> Query {
    Parser::parse(sql).expect("test: valid VelesQL")
}

/// The first row of the `INSERT` in `sql`, bound to `params`.
fn insert_row(sql: &str, params: &Params) -> Result<BoundInsertRow> {
    let query = parse(sql);
    let Some(crate::velesql::DmlStatement::Insert(stmt)) = query.dml.as_ref() else {
        panic!("test: `{sql}` is an INSERT");
    };
    bind_insert_row(&stmt.columns, &stmt.rows[0], params)
}

/// The `SET` list of the `UPDATE` in `sql`, bound to `params`.
fn assignments(sql: &str, params: &Params) -> Result<BoundAssignments> {
    let query = parse(sql);
    let Some(crate::velesql::DmlStatement::Update(stmt)) = query.dml.as_ref() else {
        panic!("test: `{sql}` is an UPDATE");
    };
    bind_update_assignments(&stmt.assignments, params)
}

fn v(value: serde_json::Value) -> Params {
    Params::from([("v".to_string(), value)])
}

fn query_error(message: &str) -> String {
    Error::Query(message.to_string()).to_string()
}

#[test]
fn an_insert_row_binds_its_id_vector_and_payload() {
    let row = insert_row(
        "INSERT INTO t (id, vector, cat) VALUES (3, $v, 'x')",
        &v(json!([1.0, 0.0])),
    )
    .expect("test: binds");
    let point = row.into_point(VECTORS).expect("test: a vector point");
    assert_eq!(point.id, 3);
    assert_eq!(point.vector, vec![1.0, 0.0]);
    assert_eq!(point.payload, Some(json!({ "cat": "x" })));
}

#[test]
fn an_insert_row_refuses_what_core_s_insert_refuses() {
    for (sql, params, column, message) in [
        (
            "INSERT INTO t (cat) VALUES ('x')",
            v(json!(null)),
            NO_VECTORS,
            "INSERT requires integer 'id' column",
        ),
        (
            "INSERT INTO t (id, cat) VALUES ('a', 'x')",
            v(json!(null)),
            NO_VECTORS,
            "id must be an integer",
        ),
        (
            "INSERT INTO t (id, cat) VALUES ($v, 'x')",
            v(json!(-1)),
            NO_VECTORS,
            "id must be >= 0",
        ),
        (
            "INSERT INTO t (id, vector) VALUES (1, NULL)",
            v(json!(null)),
            VECTORS,
            "'vector' must be an array of numbers",
        ),
    ] {
        let err = insert_row(sql, &params)
            .and_then(|row| row.into_point(column))
            .expect_err("test: refused");
        assert_eq!(err.to_string(), query_error(message), "{sql}");
    }
}

/// A row whose cells do not match its columns is refused, not zipped short:
/// the parser refuses such a statement, but an engine handed a statement
/// built another way (the WASM executor's, a binding's) used to bind the
/// cells that paired up and drop the rest — a dropped `vector` cell wrote a
/// row without a vector.
#[test]
fn a_row_whose_cells_do_not_match_its_columns_is_refused() {
    let columns = ["id".to_string(), "vector".to_string()];
    for row in [
        vec![Value::Integer(1)],
        vec![Value::Integer(1), Value::Null, Value::Null],
    ] {
        let err = bind_insert_row(&columns, &row, &v(json!(null))).expect_err("test: refused");
        assert_eq!(
            err.to_string(),
            query_error(&format!(
                "INSERT row has {} values but 2 columns were declared",
                row.len()
            ))
        );
    }
}

/// A vector written where none is stored is refused as the collection API
/// refuses it (`VELES-014`), on a metadata collection and a graph without
/// embeddings alike, by INSERT and UPDATE.
#[test]
fn a_vector_where_none_is_stored_is_refused_with_veles_014() {
    for column in [NO_VECTORS, STORES_NONE.vector_column(true)] {
        let inserted = insert_row(
            "INSERT INTO t (id, vector) VALUES (1, $v)",
            &v(json!([1.0])),
        )
        .and_then(|row| row.into_point(column))
        .expect_err("test: refused");
        assert_eq!(
            inserted.to_string(),
            Error::VectorNotAllowed("t".into()).to_string()
        );
        let set = assignments("UPDATE t SET vector = $v", &v(json!([1.0]))).expect("test");
        let row = crate::Point::metadata_only(1, json!({}));
        let updated = set.apply(&row, column).expect_err("test: refused");
        assert_eq!(updated.code(), "VELES-014");
    }
}

#[test]
fn update_assignments_bind_before_any_row_and_refuse_the_id() {
    let unbound =
        assignments("UPDATE t SET vector = $w", &v(json!(null))).expect_err("test: $w is unbound");
    assert_eq!(unbound.code(), "VELES-038");
    let id = assignments("UPDATE t SET id = 3", &v(json!(null))).expect_err("test: id");
    assert_eq!(
        id.to_string(),
        query_error("UPDATE cannot modify primary key column 'id'")
    );
}

#[test]
fn update_assignments_apply_to_a_point_as_core_s_update_does() {
    let set = assignments("UPDATE t SET vector = $v, cat = 'y'", &v(json!([0.0, 1.0])))
        .expect("test: binds");
    let point = crate::Point::new(1, vec![1.0, 0.0], Some(json!({ "cat": "x", "n": 1 })));
    let updated = set.apply(&point, VECTORS).expect("test: a vector point");
    assert_eq!(updated.vector, vec![0.0, 1.0]);
    assert_eq!(updated.payload, Some(json!({ "cat": "y", "n": 1 })));

    let row = crate::Point::metadata_only(1, json!({}));
    let fields = assignments("UPDATE t SET cat = 'z'", &v(json!(null))).expect("test: binds");
    let updated = fields
        .apply(&row, NO_VECTORS)
        .expect("test: a payload update");
    assert_eq!(updated.payload, Some(json!({ "cat": "z" })));
    assert!(updated.vector.is_empty());
}

/// A graph with embeddings: a row may carry a vector or not. Without one it
/// is a node with an empty vector, the collection's write stores it as a
/// payload; `SET vector` on such a node attaches one.
#[test]
fn an_optional_vector_column_takes_rows_with_and_without_one() {
    let none = insert_row("INSERT INTO g (id, cat) VALUES (1, 'x')", &v(json!(null)))
        .and_then(|row| row.into_point(EMBEDDINGS))
        .expect("test: a payload-only node");
    assert!(none.vector.is_empty());
    assert_eq!(none.payload, Some(json!({ "cat": "x" })));
    let some = insert_row(
        "INSERT INTO g (id, vector) VALUES (2, $v)",
        &v(json!([1.0, 0.0])),
    )
    .and_then(|row| row.into_point(EMBEDDINGS))
    .expect("test: an embedded node");
    assert_eq!(some.vector, vec![1.0, 0.0]);

    let attach = assignments("UPDATE g SET vector = $v", &v(json!([0.0, 1.0]))).expect("test");
    let updated = attach.apply(&none, EMBEDDINGS).expect("test: attaches");
    assert_eq!(updated.vector, vec![0.0, 1.0]);
    let fields = assignments("UPDATE g SET cat = 'y'", &v(json!(null))).expect("test");
    let kept = fields.apply(&none, EMBEDDINGS).expect("test: payload only");
    assert!(
        kept.vector.is_empty(),
        "a payload update attaches no vector"
    );
}

#[test]
fn the_column_follows_what_the_collection_stores() {
    let none_graph = STORES_NONE.vector_column(true);
    for (column, required, forbidden) in [
        (VECTORS, true, false),
        (EMBEDDINGS, false, false),
        (NO_VECTORS, false, true),
        (none_graph, false, true),
    ] {
        assert_eq!(column.is_required(), required, "{column:?}");
        assert_eq!(column.is_forbidden(), forbidden, "{column:?}");
    }
}

/// A bound `vector` is a vector of the collection's dimension, under every
/// column that takes one. An empty one is no "no vector" in a graph with
/// embeddings — it used to write a node without an embedding on INSERT and
/// keep the old one on UPDATE — so it is a vector of the wrong dimension
/// there (`VELES-004`); on a vector collection, whose every row stores a
/// vector, it is a row without one (`VELES-016`), as the collection API
/// refuses it.
#[test]
fn a_bound_vector_has_the_collection_s_dimension() {
    for (column, vector, code) in [
        (VECTORS, json!([]), "VELES-016"),
        (VECTORS, json!([1.0, 0.0, 0.0]), "VELES-004"),
        (EMBEDDINGS, json!([]), "VELES-004"),
        (EMBEDDINGS, json!([1.0, 0.0, 0.0]), "VELES-004"),
    ] {
        let inserted = insert_row(
            "INSERT INTO g (id, vector) VALUES (1, $v)",
            &v(vector.clone()),
        )
        .and_then(|row| row.into_point(column));
        assert_eq!(
            inserted.expect_err("test: refused").code(),
            code,
            "{column:?} {vector}"
        );
        let set = assignments("UPDATE g SET vector = $v", &v(vector.clone())).expect("test");
        let point = crate::Point::new(1, vec![1.0, 0.0], None);
        assert_eq!(
            set.apply(&point, column).expect_err("test: refused").code(),
            code,
            "{column:?} {vector}"
        );
    }
}

/// A vector collection's row without a `vector` column is a row without a
/// vector, refused as the collection API refuses one (`VELES-016`).
#[test]
fn an_insert_row_without_a_vector_on_a_vector_collection_is_refused_with_veles_016() {
    let err = insert_row("INSERT INTO t (id, cat) VALUES (1, 'x')", &v(json!(null)))
        .and_then(|row| row.into_point(VECTORS))
        .expect_err("test: refused");
    assert_eq!(
        err.to_string(),
        Error::VectorRequired("t".to_string()).to_string()
    );
}
