use super::*;
use crate::database::DatabaseInner;
use crate::velesql_value::parse_params;
use velesdb_core::velesql::Parser;

fn parse_query(sql: &str) -> Query {
    Parser::parse(sql).expect("test: parse")
}

fn seed_metadata_docs(db: &mut DatabaseInner) {
    db.create_metadata_collection("docs").expect("test: create");
    let store = db.get_shared_store("docs").expect("test: store");
    let mut borrowed = store.borrow_mut();
    for (id, cat) in [(1u64, "tech"), (2, "food"), (3, "tech")] {
        borrowed.ids.push(id);
        borrowed
            .payloads
            .push(Some(serde_json::json!({"cat": cat})));
    }
}

fn seed_vector_collection(db: &mut DatabaseInner) {
    db.create_collection("vecs", 4, "cosine")
        .expect("test: create");
    let store = db.get_shared_store("vecs").expect("test: store");
    for (id, v) in [
        (10u64, vec![1.0, 0.0, 0.0, 0.0]),
        (11, vec![0.0, 1.0, 0.0, 0.0]),
        (12, vec![0.0, 0.0, 1.0, 0.0]),
    ] {
        crate::store_insert::insert_with_payload(
            &mut store.borrow_mut(),
            id,
            &v,
            Some(serde_json::json!({"cat": if id == 10 { "a" } else { "b" }})),
        );
    }
}

#[test]
fn test_select_all_returns_all_rows() {
    let mut db = DatabaseInner::new();
    seed_metadata_docs(&mut db);
    let q = parse_query("SELECT * FROM docs");
    let rows = execute(&mut db, &q, &parse_params(None).expect("test: p")).expect("test: select");
    assert_eq!(rows.len(), 3);
}

#[test]
fn test_select_with_limit() {
    let mut db = DatabaseInner::new();
    seed_metadata_docs(&mut db);
    let q = parse_query("SELECT * FROM docs LIMIT 2");
    let rows = execute(&mut db, &q, &parse_params(None).expect("test: p")).expect("test: select");
    assert_eq!(rows.len(), 2);
}

#[test]
fn test_select_where_filters() {
    let mut db = DatabaseInner::new();
    seed_metadata_docs(&mut db);
    let q = parse_query("SELECT * FROM docs WHERE cat = 'tech'");
    let rows = execute(&mut db, &q, &parse_params(None).expect("test: p")).expect("test: select");
    assert_eq!(rows.len(), 2);
}

#[test]
fn test_select_near_returns_ranked_results() {
    let mut db = DatabaseInner::new();
    seed_vector_collection(&mut db);
    let q = parse_query("SELECT * FROM vecs WHERE vector NEAR $q LIMIT 2");
    let params = parse_params(Some(r#"{"q": [1.0, 0.0, 0.0, 0.0]}"#)).expect("test: p");
    let rows = execute(&mut db, &q, &params).expect("test: near");
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].id(), 10);
}

#[test]
fn test_select_near_dimension_mismatch_errors() {
    let mut db = DatabaseInner::new();
    seed_vector_collection(&mut db);
    let q = parse_query("SELECT * FROM vecs WHERE vector NEAR $q LIMIT 2");
    let params = parse_params(Some(r#"{"q": [1.0, 0.0]}"#)).expect("test: p");
    let err = execute(&mut db, &q, &params);
    assert!(err.is_err());
}

// --- Finding F10: O(n) id lookup scales linearly, not quadratically ---
//
// Regression test: with 500 rows, the previous O(n^2) `.position(...)`
// path did 500 * 500 = 250_000 id comparisons per NEAR query. The
// hash-map path does ~500 lookups. This test asserts correctness at a
// scale large enough that a future regression to `.position(...)` would
// visibly slow the suite; the numbers themselves are not the point —
// correctness is.

#[test]
fn test_select_near_scales_with_hashmap_lookup() {
    let mut db = DatabaseInner::new();
    db.create_collection("vecs_large", 4, "cosine")
        .expect("test: create");
    let store = db.get_shared_store("vecs_large").expect("test: store");
    for i in 0u64..500 {
        #[allow(clippy::cast_precision_loss)]
        let val = i as f32;
        crate::store_insert::insert_with_payload(
            &mut store.borrow_mut(),
            i,
            &[val, 0.0, 0.0, 0.0],
            None,
        );
    }
    drop(store);
    let q = parse_query("SELECT * FROM vecs_large WHERE vector NEAR $q LIMIT 10");
    let params = parse_params(Some(r#"{"q": [1.0, 0.0, 0.0, 0.0]}"#)).expect("test: p");
    let rows = execute(&mut db, &q, &params).expect("test: near");
    // With cosine, only rows with strictly positive first component
    // match; row id=0 has a zero-norm vector and returns NaN score
    // (filtered out). We assert we got 10 rows (LIMIT) and that they
    // are all from the seeded collection (id < 500).
    assert_eq!(rows.len(), 10);
    for row in &rows {
        assert!(row.id() < 500, "id should come from seeded range");
    }
}
