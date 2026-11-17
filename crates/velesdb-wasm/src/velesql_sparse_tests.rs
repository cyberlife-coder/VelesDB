//! `SPARSE_NEAR` in the WASM executor: the refusals and the shapes the core
//! parity suite (`velesql_sparse_parity_tests.rs`) does not cover.

use crate::database::DatabaseInner;
use crate::velesql_exec::execute;

fn db(with_sparse: bool) -> DatabaseInner {
    let mut db = DatabaseInner::new();
    db.create_collection("c", 2, "cosine")
        .expect("test: create");
    for (id, v, cat) in [(1u64, "[1.0, 0.0]", "a"), (2, "[0.0, 1.0]", "b")] {
        execute(
            &mut db,
            &format!("INSERT INTO c (id, vector, cat) VALUES ({id}, $v, '{cat}')"),
            Some(&format!("{{\"v\": {v}}}")),
        )
        .expect("test: insert");
    }
    if with_sparse {
        let store = db.get_shared_store("c").expect("test: store");
        store
            .borrow_mut()
            .sparse_insert(2, &[7], &[1.0])
            .expect("test: sparse");
    }
    db
}

fn ids(db: &mut DatabaseInner, sql: &str) -> Result<Vec<u64>, String> {
    let r = execute(db, sql, Some(r#"{"q": [1.0, 0.0]}"#))?;
    Ok((0..r.row_count() as usize)
        .map(|i| r.row(i).expect("test: row").id())
        .collect())
}

/// Without a sparse index there is nothing to search: refused with core's
/// words, not answered as if the sparse leaf were absent.
#[test]
fn a_collection_without_sparse_vectors_refuses_sparse_near() {
    for sql in [
        "SELECT * FROM c WHERE vector SPARSE_NEAR {7: 1.0} LIMIT 5",
        "SELECT * FROM c WHERE vector NEAR $q AND vector SPARSE_NEAR {7: 1.0} LIMIT 5",
    ] {
        let err = ids(&mut db(false), sql).expect_err("no sparse index");
        assert!(
            err.contains("Sparse index '<default>' not found"),
            "{sql}: {err}"
        );
    }
}

/// The browser store holds one, unnamed, sparse index.
#[test]
fn a_named_sparse_index_is_not_found() {
    let err = ids(
        &mut db(true),
        "SELECT * FROM c WHERE vector SPARSE_NEAR {7: 1.0} USING 'title' LIMIT 5",
    )
    .expect_err("no named index");
    assert!(err.contains("Sparse index 'title' not found"), "{err}");
}

#[test]
fn a_non_finite_sparse_parameter_is_refused_as_core_refuses_it() {
    let err = execute(
        &mut db(true),
        "SELECT * FROM c WHERE vector SPARSE_NEAR $s LIMIT 5",
        Some(r#"{"s": {"7": 1e300}}"#),
    )
    .expect_err("an infinite weight");
    assert!(err.contains("not finite"), "{err}");
}

/// An aggregation reads every point its metadata filter keeps, as core's
/// `execute_aggregate` does: a `SPARSE_NEAR` leaf filters no point there.
#[test]
fn an_aggregate_over_sparse_near_counts_every_point() {
    let r = execute(
        &mut db(true),
        "SELECT COUNT(*) FROM c WHERE vector SPARSE_NEAR {7: 1.0}",
        None,
    )
    .expect("test: count");
    assert!(r.rows_json().contains("\"count\":2"), "{}", r.rows_json());
}

/// Point 2 is second on the dense side but alone on the sparse side, so the
/// fusion ranks it first; the dense ranking alone puts point 1 first.
#[test]
fn a_hybrid_ranks_by_the_fusion() {
    let mut db = db(true);
    let sql = "SELECT * FROM c WHERE vector NEAR $q AND vector SPARSE_NEAR {7: 1.0} LIMIT 5";
    assert_eq!(ids(&mut db, sql).expect("test: hybrid"), vec![2, 1]);
}
