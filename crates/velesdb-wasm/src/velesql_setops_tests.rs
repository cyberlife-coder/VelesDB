//! Compound queries through the whole WASM executor. The combinators
//! themselves are core's (`velesdb_core::velesql::set_operation`, unit-tested
//! there); these pin the wiring: operands, projection, window and refusals.

use crate::database::DatabaseInner;
use crate::velesql_exec::execute;

/// `alpha`: ids 1, 2, 3 (`cat = 'tech'`); `beta`: ids 3, 4, 5
/// (`cat = 'science'`) — point 3 is in both, with different payloads.
fn db() -> DatabaseInner {
    let mut db = DatabaseInner::new();
    for (name, ids, cat) in [("alpha", [1, 2, 3], "tech"), ("beta", [3, 4, 5], "science")] {
        db.create_collection(name, 2, "cosine")
            .expect("test: create");
        for id in ids {
            execute(
                &mut db,
                &format!("INSERT INTO {name} (id, vector, cat) VALUES ({id}, $v, '{cat}')"),
                Some(r#"{"v": [1.0, 0.0]}"#),
            )
            .expect("test: seed");
        }
    }
    db
}

fn ids(db: &mut DatabaseInner, sql: &str) -> Vec<u64> {
    let result = execute(db, sql, None).unwrap_or_else(|e| panic!("test: {sql}: {e}"));
    (0..result.row_count() as usize)
        .map(|i| result.row(i).expect("test: row").id())
        .collect()
}

#[test]
fn a_star_row_is_its_id_and_payload() {
    let mut db = db();
    let union = ids(&mut db, "SELECT * FROM alpha UNION SELECT * FROM beta");
    assert_eq!(union.len(), 6, "the two points 3 differ: {union:?}");
    let by_id = ids(
        &mut db,
        "SELECT id FROM alpha UNION SELECT id FROM beta ORDER BY id ASC",
    );
    assert_eq!(by_id, vec![1, 2, 3, 4, 5]);
}

#[test]
fn intersect_and_except_compare_projected_values() {
    let mut db = db();
    assert_eq!(
        ids(
            &mut db,
            "SELECT id FROM alpha INTERSECT SELECT id FROM beta"
        ),
        vec![3]
    );
    assert!(ids(&mut db, "SELECT * FROM alpha INTERSECT SELECT * FROM beta").is_empty());
    assert_eq!(
        ids(
            &mut db,
            "SELECT id FROM alpha EXCEPT SELECT id FROM beta ORDER BY id ASC"
        ),
        vec![1, 2]
    );
}

/// The clauses after the last operand window the compound; a first
/// operand's clauses only its own rows.
#[test]
fn the_last_operands_clauses_window_the_compound() {
    let mut db = db();
    assert_eq!(
        ids(
            &mut db,
            "SELECT id FROM alpha UNION SELECT id FROM beta ORDER BY id ASC LIMIT 2 OFFSET 2"
        ),
        vec![3, 4]
    );
    assert_eq!(
        ids(
            &mut db,
            "SELECT id FROM alpha ORDER BY id ASC OFFSET 2 UNION SELECT id FROM beta ORDER BY id ASC"
        ),
        vec![3, 4, 5]
    );
}

#[test]
fn an_operand_that_joins_is_refused() {
    let mut db = db();
    let err = execute(
        &mut db,
        "SELECT * FROM alpha JOIN beta ON alpha.id = beta.id UNION SELECT * FROM beta",
        None,
    )
    .map(|_| ())
    .expect_err("test: refused");
    assert!(err.contains("UNION / INTERSECT / EXCEPT operand"), "{err}");
}

/// `ORDER BY similarity(vector, $q)` over a compound reads each row's own
/// vector, from the collection it came from (ids are collection-local).
#[test]
fn the_windows_similarity_order_reads_each_rows_own_vector() {
    let mut db = DatabaseInner::new();
    for (name, id, v) in [
        ("a", 1, "[0.0, 1.0]"),
        ("b", 2, "[1.0, 0.0]"),
        ("c", 1, "[1.0, 0.0]"),
    ] {
        db.create_collection(name, 2, "cosine")
            .expect("test: create");
        execute(
            &mut db,
            &format!("INSERT INTO {name} (id, vector, src) VALUES ({id}, $v, '{name}')"),
            Some(&format!(r#"{{"v": {v}}}"#)),
        )
        .expect("test: seed");
    }
    let nearest = |db: &mut DatabaseInner, other: &str| {
        let sql = format!(
            "SELECT * FROM a UNION ALL SELECT * FROM {other} \
             ORDER BY similarity(vector, $q) DESC LIMIT 1"
        );
        let result = execute(db, &sql, Some(r#"{"q": [1.0, 0.0]}"#))
            .unwrap_or_else(|e| panic!("test: {sql}: {e}"));
        let row = result.row(0).expect("test: row");
        let data: serde_json::Value =
            serde_json::from_str(&row.data_json()).expect("test: row json");
        (
            row.id(),
            data["src"].as_str().expect("test: src").to_string(),
        )
    };
    assert_eq!(
        nearest(&mut db, "b"),
        (2, "b".to_string()),
        "b's id 2 is absent from a"
    );
    assert_eq!(
        nearest(&mut db, "c"),
        (1, "c".to_string()),
        "c's point 1, not a's"
    );
}

/// A compound's columns are positional and named by the first operand: the
/// second operand's `age` comes back under `year`.
#[test]
fn a_later_operands_column_comes_back_under_the_first_operands_name() {
    let mut db = DatabaseInner::new();
    for (name, column, value) in [("a", "year", 2020), ("b", "age", 30)] {
        db.create_collection(name, 2, "cosine")
            .expect("test: create");
        execute(
            &mut db,
            &format!("INSERT INTO {name} (id, vector, {column}) VALUES (1, $v, {value})"),
            Some(r#"{"v": [1.0, 0.0]}"#),
        )
        .expect("test: seed");
    }
    let result = execute(
        &mut db,
        "SELECT year FROM a UNION ALL SELECT age FROM b ORDER BY year ASC",
        None,
    )
    .expect("test: compound");
    let years: Vec<serde_json::Value> = (0..result.row_count() as usize)
        .map(|i| {
            let data: serde_json::Value =
                serde_json::from_str(&result.row(i).expect("test: row").data_json())
                    .expect("test: json");
            data["year"].clone()
        })
        .collect();
    assert_eq!(years, [serde_json::json!(30), serde_json::json!(2020)]);
}
