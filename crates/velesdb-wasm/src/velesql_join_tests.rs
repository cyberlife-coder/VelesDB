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
    let rows = execute(&db, &stmt, &Params::new()).expect("test: join");
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
    let rows = execute(&db, &stmt, &Params::new()).expect("test: left join");
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
    let rows = execute(&db, &stmt, &Params::new()).expect("test: join where");
    assert_eq!(rows.len(), 2);
}

#[test]
fn test_join_missing_right_collection_errors() {
    let mut db = DatabaseInner::new();
    db.create_metadata_collection("users").expect("test: users");
    let stmt = parse("SELECT * FROM users JOIN ghost ON users.id = ghost.user_id LIMIT 10");
    let err = execute(&db, &stmt, &Params::new());
    assert!(err.is_err());
}

#[test]
fn test_right_join_is_rejected() {
    let mut db = DatabaseInner::new();
    seed(&mut db);
    let stmt = parse("SELECT * FROM users RIGHT JOIN orders ON users.id = orders.user_id LIMIT 10");
    let err = execute(&db, &stmt, &Params::new());
    assert!(err.is_err());
    assert!(err.expect_err("test: err").contains("RIGHT JOIN"));
}
