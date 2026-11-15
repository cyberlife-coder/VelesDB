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
