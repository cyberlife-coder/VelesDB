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
fn joined_window_distinguishes_a_literal_sort_key_from_a_projected_path() {
    let mut db = DatabaseInner::new();
    seed(&mut db);
    let orders = db.get_shared_store("orders").unwrap();
    for payload in orders.borrow_mut().payloads.iter_mut().flatten() {
        payload["emb"] = serde_json::json!([1, 0]);
        payload["o.total"] = serde_json::json!(-payload["total"].as_f64().unwrap());
        payload["u.name"] = serde_json::json!("literal");
    }
    let sql = "SELECT o.total AS child, u.name, ROW_NUMBER() OVER (ORDER BY o.total) AS o \
        FROM users u JOIN orders o ON u.id=o.user_id ORDER BY similarity(o.emb, [1, 0]) DESC, o.total ASC";
    let result = crate::velesql_exec::execute(&mut db, sql, None).unwrap();
    let rows: Vec<serde_json::Value> = serde_json::from_str(&result.rows_json()).unwrap();
    assert_eq!(
        rows.iter()
            .map(|row| row["child"].as_f64())
            .collect::<Vec<_>>(),
        [Some(75.0), Some(50.0), Some(20.0)]
    );
    assert_eq!(
        rows.iter()
            .map(|row| row["u.name"].as_str())
            .collect::<Vec<_>>(),
        [Some("Alice"), Some("Alice"), Some("Bob")]
    );
}

#[test]
fn joined_window_reads_every_sibling_before_publishing_aliases() {
    let mut db = DatabaseInner::new();
    seed(&mut db);
    let users = db.get_shared_store("users").unwrap();
    let mut users = users.borrow_mut();
    users.ids.push(3);
    users
        .payloads
        .push(Some(serde_json::json!({"name":"Carol"})));
    for (i, payload) in users.payloads.iter_mut().flatten().enumerate() {
        payload["meta"] = serde_json::json!({"val":21+i});
        payload["keep"] = serde_json::json!(31 + i);
    }
    drop(users);
    for order in [
        "ORDER BY similarity(o.emb,[1,0]) DESC",
        "ORDER BY id ASC, o.total ASC",
        "",
    ] {
        let sql = format!(
            "SELECT id, o.total AS child, o.total AS u, u.name, o.total AS meta, meta.val, \
               o.total AS keep, keep AS saved, ROW_NUMBER() OVER (ORDER BY o.total) AS o \
               FROM users u LEFT JOIN orders o ON u.id=o.user_id {order}"
        );
        let result = crate::velesql_exec::execute(&mut db, &sql, None).unwrap();
        let rows: Vec<serde_json::Value> = serde_json::from_str(&result.rows_json()).unwrap();
        assert_eq!(rows.len(), 4);
        for row in rows {
            let id = row["id"].as_u64().unwrap();
            assert_eq!(
                row["u.name"],
                ["Alice", "Bob", "Carol"][usize::try_from(id - 1).unwrap()],
                "{row}"
            );
            assert_eq!(row["meta.val"], 20 + id, "{row}");
            assert_eq!(row["saved"], 30 + id, "{row}");
            assert_eq!(row["child"].is_null(), id == 3);
            assert_eq!(row["u"], row["child"]);
            assert_eq!(row["meta"], row["child"]);
            assert_eq!(row["keep"], row["child"]);
            assert!(row["o"].is_u64());
        }
    }
}

#[test]
fn joined_window_projects_the_qualified_side_field() {
    let mut db = DatabaseInner::new();
    seed(&mut db);
    let users = db.get_shared_store("users").expect("test: users");
    users.borrow_mut().ids.push(3);
    users
        .borrow_mut()
        .payloads
        .push(Some(serde_json::json!({"name":"Carol"})));
    let sql = "SELECT o.total AS child, total, ROW_NUMBER() OVER (ORDER BY o.total) AS o \
               FROM users u LEFT JOIN orders o ON u.id = o.user_id \
               ORDER BY similarity(o.emb, [1, 0]) DESC";
    let result = crate::velesql_exec::execute(&mut db, sql, None).expect("test: joined projection");
    let rows: Vec<serde_json::Value> =
        serde_json::from_str(&result.rows_json()).expect("test: rows");
    assert_eq!(rows.len(), 4);
    let values: Vec<_> = rows
        .iter()
        .filter_map(|row| row["child"].as_f64())
        .collect();
    assert_eq!(values.len(), 3);
    assert!(rows.iter().any(|row| row["child"].is_null()));
    for row in rows {
        assert_eq!(row["child"], row["total"]);
        assert!(row["o"].is_u64());
        assert!(row.get("o.emb").is_none());
    }
}

#[test]
fn joined_window_orders_by_the_qualified_side_field() {
    let mut db = DatabaseInner::new();
    seed(&mut db);
    let orders = db.get_shared_store("orders").expect("test: orders");
    for payload in orders.borrow_mut().payloads.iter_mut().flatten() {
        payload["emb"] = serde_json::json!([1, 0]);
    }
    for (direction, totals) in [("ASC", [20.0, 50.0, 75.0]), ("DESC", [75.0, 50.0, 20.0])] {
        for alias in ["rn", "o"] {
            let sql = format!(
                "SELECT total, ROW_NUMBER() OVER (ORDER BY o.total) AS {alias} \
                 FROM users u JOIN orders o ON u.id = o.user_id ORDER BY \
                 similarity(o.emb, [1, 0]) DESC, o.total {direction}, ({alias} * 2) DESC"
            );
            let result = crate::velesql_exec::execute(&mut db, &sql, None).expect("test: order");
            let rows: Vec<serde_json::Value> =
                serde_json::from_str(&result.rows_json()).expect("test: rows");
            let values: Vec<_> = rows
                .iter()
                .map(|row| row["total"].as_f64().expect("test: total"))
                .collect();
            assert_eq!(values, totals, "{sql}");
        }
    }
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
    let ids: Vec<u64> = rows.iter().map(|r| r.id).collect();
    assert_eq!(ids, [1, 1, 2, 13]);
    let unmatched = rows[3].payload.as_ref().expect("test: payload");
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
        .find(|r| r.id == 1)
        .and_then(|r| r.payload.as_ref())
        .expect("test: Ann's row");
    assert_eq!(ann["e"]["name"], serde_json::json!("Ann"));
    assert_eq!(ann["m"]["name"], serde_json::json!("Bo"));
}
