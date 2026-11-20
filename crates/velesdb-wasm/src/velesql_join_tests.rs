use super::*;
use velesdb_core::velesql::Parser;

fn seed(db: &mut DatabaseInner) {
    db.create_metadata_collection("users").expect("test: users");
    let users = db.get_shared_store("users").expect("test: users store");
    let mut ub = users.borrow_mut();
    for (id, name) in [(1u64, "Alice"), (2, "Bob")] {
        ub.ids.push(id);
        ub.payloads.push(Some(serde_json::json!({"name": name})));
    }
    drop(ub);

    db.create_metadata_collection("orders")
        .expect("test: orders");
    let orders = db.get_shared_store("orders").expect("test: orders store");
    let mut ob = orders.borrow_mut();
    for (id, uid, total) in [(10u64, 1u64, 50.0f64), (11, 1, 75.0), (12, 2, 20.0)] {
        ob.ids.push(id);
        ob.payloads
            .push(Some(serde_json::json!({"user_id": uid, "total": total})));
    }
}

fn parse(sql: &str) -> SelectStatement {
    Parser::parse(sql).expect("test: parse").select
}

#[test]
fn test_inner_join_equality() {
    let mut db = DatabaseInner::new();
    seed(&mut db);
    let stmt = parse("SELECT * FROM users JOIN orders ON users.id = orders.user_id LIMIT 10");
    let rows = joined_rows(&db, &stmt, &Params::new()).expect("test: join");
    assert_eq!(rows.len(), 3); // alice:2 orders + bob:1
}

#[test]
fn test_left_join_preserves_unmatched_left() {
    let mut db = DatabaseInner::new();
    seed(&mut db);
    // Insert a user with no orders.
    db.create_metadata_collection("lonely")
        .expect("test: lonely");
    // Actually easier: add a 3rd user and verify.
    let users = db.get_shared_store("users").expect("test: users");
    let mut ub = users.borrow_mut();
    ub.ids.push(3);
    ub.payloads.push(Some(serde_json::json!({"name": "Carol"})));
    drop(ub);

    let stmt = parse("SELECT * FROM users LEFT JOIN orders ON users.id = orders.user_id LIMIT 10");
    let rows = joined_rows(&db, &stmt, &Params::new()).expect("test: left join");
    // 2 for Alice + 1 for Bob + 1 null-padded for Carol = 4
    assert_eq!(rows.len(), 4);
}

#[test]
fn test_join_with_where_filter() {
    let mut db = DatabaseInner::new();
    seed(&mut db);
    let stmt = parse(
        "SELECT * FROM users JOIN orders ON users.id = orders.user_id WHERE name = 'Alice' LIMIT 10",
    );
    let rows = joined_rows(&db, &stmt, &Params::new()).expect("test: join where");
    assert_eq!(rows.len(), 2);
}

#[test]
fn test_join_missing_right_collection_errors() {
    let mut db = DatabaseInner::new();
    db.create_metadata_collection("users").expect("test: users");
    let stmt = parse("SELECT * FROM users JOIN ghost ON users.id = ghost.user_id LIMIT 10");
    let err = joined_rows(&db, &stmt, &Params::new());
    assert!(err.is_err());
}

/// RIGHT JOIN runs core's algorithm: the order no user matches is a row of
/// its own, whose base side reads a `null` id.
#[test]
fn a_right_join_keeps_the_unmatched_joined_rows() {
    let mut db = DatabaseInner::new();
    seed(&mut db);
    let orders = db.get_shared_store("orders").expect("test: orders store");
    let mut ob = orders.borrow_mut();
    ob.ids.push(13);
    ob.payloads
        .push(Some(serde_json::json!({"user_id": 9, "total": 5.0})));
    drop(ob);

    let stmt = parse("SELECT * FROM users RIGHT JOIN orders ON users.id = orders.user_id");
    let rows = joined_rows(&db, &stmt, &Params::new()).expect("test: right join");
    let ids: Vec<u64> = rows.iter().map(|r| r.0).collect();
    assert_eq!(ids, [1, 1, 2, 13]);
    let unmatched = rows[3].2.as_ref().expect("test: payload");
    assert_eq!(unmatched["users"]["id"], serde_json::Value::Null);
    assert_eq!(unmatched["orders"]["id"], serde_json::json!(13));
}

/// Each side reads under its alias, so a self-join keeps both names.
#[test]
fn a_self_join_keeps_each_sides_fields_under_its_alias() {
    let mut db = DatabaseInner::new();
    db.create_metadata_collection("emp").expect("test: emp");
    let emp = db.get_shared_store("emp").expect("test: emp store");
    let mut eb = emp.borrow_mut();
    for (id, name, boss) in [(1u64, "Ann", 2u64), (2, "Bo", 2)] {
        eb.ids.push(id);
        eb.payloads
            .push(Some(serde_json::json!({"name": name, "boss": boss})));
    }
    drop(eb);
    let stmt = parse("SELECT * FROM emp AS e JOIN emp AS m ON e.boss = m.id LIMIT 10");
    let rows = joined_rows(&db, &stmt, &Params::new()).expect("test: self-join");
    let ann = rows
        .iter()
        .find(|(id, _, _)| *id == 1)
        .and_then(|(_, _, p)| p.as_ref())
        .expect("test: Ann's row");
    assert_eq!(ann["e"]["name"], serde_json::json!("Ann"));
    assert_eq!(ann["m"]["name"], serde_json::json!("Bo"));
}

/// An unaliased FROM is read under its table name beside an aliased JOIN:
/// `from_alias` lists the JOIN alias too, so its first entry is no base.
#[test]
fn an_unaliased_base_reads_under_its_table_beside_an_aliased_join() {
    let mut db = DatabaseInner::new();
    seed(&mut db);
    let stmt = parse("SELECT * FROM users JOIN orders AS o ON users.id = o.user_id");
    let rows = joined_rows(&db, &stmt, &Params::new()).expect("test: join");
    let sides: Vec<(serde_json::Value, serde_json::Value)> = rows
        .iter()
        .map(|r| {
            let payload = r.2.as_ref().expect("test: payload");
            (payload["users"]["id"].clone(), payload["o"]["id"].clone())
        })
        .collect();
    let expected: Vec<(serde_json::Value, serde_json::Value)> = [(1, 10), (1, 11), (2, 12)]
        .into_iter()
        .map(|(u, o)| (serde_json::json!(u), serde_json::json!(o)))
        .collect();
    assert_eq!(sides, expected);
}
