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
    let r = execute(db, sql, Some(r#"{"q": [1.0, 0.0]}"#)).map_err(|e| e.to_string())?;
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
    assert!(err.to_string().contains("not finite"), "{err}");
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

/// Points 1 (`a`, the query's own vector), 2 (`a`, similarity 0.8) and 3
/// (`b`, similarity 0), whose sparse scores rank 1 > 2 > 3.
fn db_with_similarities() -> DatabaseInner {
    let mut db = DatabaseInner::new();
    db.create_collection("s", 2, "cosine")
        .expect("test: create");
    let points = [
        (1u64, "[1.0, 0.0]", "a", 3.0),
        (2, "[0.8, 0.6]", "a", 2.0),
        (3, "[0.0, 1.0]", "b", 1.0),
    ];
    for (id, v, cat, weight) in points {
        execute(
            &mut db,
            &format!("INSERT INTO s (id, vector, cat) VALUES ({id}, $v, '{cat}')"),
            Some(&format!("{{\"v\": {v}}}")),
        )
        .expect("test: insert");
        let store = db.get_shared_store("s").expect("test: store");
        store
            .borrow_mut()
            .sparse_insert(id, &[7], &[weight])
            .expect("test: sparse");
    }
    db
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn sparse_rows_are_ranked_before_the_final_limit() {
    let mut db = db_with_similarities();
    let rows = execute(
        &mut db,
        "SELECT id, ROW_NUMBER() OVER (ORDER BY id DESC) AS rn FROM s \
         WHERE vector SPARSE_NEAR {7: 1.0} LIMIT 1",
        None,
    )
    .expect("test: sparse window");
    let projected: serde_json::Value =
        serde_json::from_str(&rows.rows_json()).expect("test: projected rows");
    assert_eq!(projected[0]["id"], 1);
    assert_eq!(projected[0]["rn"], 3);
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn an_external_order_can_select_the_last_sparse_hit() {
    let mut db = db_with_similarities();
    for search in [
        "vector SPARSE_NEAR {7: 1.0}",
        "vector NEAR $q AND vector SPARSE_NEAR {7: 1.0}",
        "vector NEAR_FUSED [$q, $q]",
    ] {
        let sql = format!("SELECT * FROM s WHERE {search} ORDER BY id DESC LIMIT 1");
        assert_eq!(ids(&mut db, &sql).expect("test: search"), [3], "{sql}");
    }
}

/// A `similarity()` threshold beside `SPARSE_NEAR` bounds the points either
/// branch may return, evaluated as the dense SELECT path evaluates it — it
/// used to fail with "similarity() threshold filters are not supported".
#[test]
fn a_similarity_threshold_bounds_a_sparse_near() {
    let mut db = db_with_similarities();
    let near = "SELECT * FROM s WHERE vector SPARSE_NEAR {7: 1.0} AND similarity(vector, $q) > 0.5 LIMIT 5";
    assert_eq!(ids(&mut db, near).expect("test: sparse"), vec![1, 2]);
    let hybrid = "SELECT * FROM s WHERE vector NEAR $q AND vector SPARSE_NEAR {7: 1.0} \
                  AND similarity(vector, $q) > 0.5 LIMIT 5";
    assert_eq!(ids(&mut db, hybrid).expect("test: hybrid"), vec![1, 2]);
}

/// Under `OR` and `NOT` the threshold composes with the other predicates,
/// and the survivors still rank by the sparse score.
#[test]
fn a_similarity_threshold_composes_under_or_and_not() {
    let mut db = db_with_similarities();
    let or = "SELECT * FROM s WHERE vector SPARSE_NEAR {7: 1.0} \
              AND (similarity(vector, $q) > 0.9 OR cat = 'b') LIMIT 5";
    assert_eq!(ids(&mut db, or).expect("test: or"), vec![1, 3]);
    let not = "SELECT * FROM s WHERE vector SPARSE_NEAR {7: 1.0} \
               AND NOT similarity(vector, $q) > 0.9 LIMIT 5";
    assert_eq!(ids(&mut db, not).expect("test: not"), vec![2, 3]);
}
