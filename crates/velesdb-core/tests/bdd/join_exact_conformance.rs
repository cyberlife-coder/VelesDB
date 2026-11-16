//! BDD tests: golden JOIN row-sets across all four join types plus chained and
//! self joins.
//!
//! The sibling `cross_collection_join_optimization.rs` only pins INNER/LEFT
//! cardinalities; this module asserts the *exact* merged row-set (ids + field
//! values + null-fill) for INNER / LEFT / RIGHT / FULL, a 3-way chained JOIN,
//! and a self-JOIN.
//!
//! Routing notes verified against the engine source:
//! - `Database::execute_single_join` (database/query_join.rs) takes the id
//!   lookup or the secondary-index shortcut only for an INNER or LEFT join
//!   with nothing pushed to the joined side; every other join, RIGHT and FULL
//!   included, runs `velesql::hash_join` over the joined collection's points.
//!   The base side joins on a non-`id` foreign-key column
//!   (`base.fk = joined.id`), so these scenarios run the hash join.
//! - Unmatched LEFT rows carry the joined collection's columns as JSON
//!   `null`; unmatched RIGHT rows are the joined point's own row — its id,
//!   its fields — and lack base-only columns.

use std::collections::HashSet;

use serde_json::json;
use velesdb_core::{Database, Point, SearchResult};

use super::helpers::{create_test_db, execute_sql, payload_f64, payload_str, result_ids};

// =========================================================================
// Helpers
// =========================================================================

/// Creates `orders` (VectorCollection) and `customers` (MetadataCollection)
/// with a partially-overlapping foreign-key relationship.
///
/// orders.customer_id -> customers.id:
///   order 1 -> 10 (match), order 2 -> 20 (match), order 3 -> 99 (no match)
/// customer 30 has no referencing order (unmatched right).
fn setup_orders_customers(db: &Database) {
    execute_sql(
        db,
        "CREATE COLLECTION orders (dimension = 4, metric = 'cosine');",
    )
    .expect("CREATE orders");

    let orders = db.get_vector_collection("orders").expect("get orders");
    orders
        .upsert(vec![
            Point::new(
                1,
                vec![1.0, 0.0, 0.0, 0.0],
                Some(json!({"title": "ord-a", "customer_id": 10})),
            ),
            Point::new(
                2,
                vec![0.0, 1.0, 0.0, 0.0],
                Some(json!({"title": "ord-b", "customer_id": 20})),
            ),
            Point::new(
                3,
                vec![0.0, 0.0, 1.0, 0.0],
                Some(json!({"title": "ord-c", "customer_id": 99})),
            ),
        ])
        .expect("upsert orders");

    execute_sql(db, "CREATE METADATA COLLECTION customers;").expect("CREATE customers");
    let customers = db
        .get_metadata_collection("customers")
        .expect("get customers");
    customers
        .upsert(vec![
            Point::metadata_only(10, json!({"name": "Cust-X"})),
            Point::metadata_only(20, json!({"name": "Cust-Y"})),
            Point::metadata_only(30, json!({"name": "Cust-Z"})),
        ])
        .expect("upsert customers");
}

/// Returns the result whose point id matches, panicking with context if absent.
fn row(results: &[SearchResult], id: u64) -> &SearchResult {
    results
        .iter()
        .find(|r| r.point.id == id)
        .unwrap_or_else(|| panic!("test: expected a row with id {id}"))
}

/// True when `field` is present and JSON `null` (the LEFT/FULL null-fill marker).
fn field_is_json_null(result: &SearchResult, field: &str) -> bool {
    result
        .point
        .payload
        .as_ref()
        .and_then(|p| p.get(field))
        .is_some_and(serde_json::Value::is_null)
}

// =========================================================================
// (1) INNER JOIN -> exact matched-pair row-set
// =========================================================================

/// GIVEN orders fk-joined to customers with one non-matching order
/// WHEN an INNER JOIN runs
/// THEN only the two matched pairs appear, each carrying both sides' fields.
#[test]
fn test_inner_join_exact_matched_pairs_only() {
    let (_dir, db) = create_test_db();
    setup_orders_customers(&db);

    let sql = "SELECT * FROM orders \
               JOIN customers ON orders.customer_id = customers.id \
               LIMIT 50";
    let results = execute_sql(&db, sql).expect("INNER fk JOIN");

    assert_eq!(result_ids(&results), HashSet::from([1, 2]));
    assert_eq!(payload_str(row(&results, 1), "name"), Some("Cust-X"));
    assert_eq!(payload_str(row(&results, 1), "title"), Some("ord-a"));
    assert_eq!(payload_str(row(&results, 2), "name"), Some("Cust-Y"));
}

// =========================================================================
// (2) LEFT JOIN -> exact row-set incl. null-fill for the unmatched left row
// =========================================================================

/// GIVEN the same fk relationship
/// WHEN a LEFT JOIN runs
/// THEN all three orders appear; the unmatched order keeps its own fields and
/// carries the joined `name` as JSON null.
#[test]
fn test_left_join_exact_with_null_fill() {
    let (_dir, db) = create_test_db();
    setup_orders_customers(&db);

    let sql = "SELECT * FROM orders \
               LEFT JOIN customers ON orders.customer_id = customers.id \
               LIMIT 50";
    let results = execute_sql(&db, sql).expect("LEFT fk JOIN");

    assert_eq!(result_ids(&results), HashSet::from([1, 2, 3]));
    assert_eq!(payload_str(row(&results, 1), "name"), Some("Cust-X"));
    // Unmatched left row 3 keeps its title, joined name is null-filled.
    assert_eq!(payload_str(row(&results, 3), "title"), Some("ord-c"));
    assert!(
        field_is_json_null(row(&results, 3), "name"),
        "unmatched LEFT row must carry joined name as JSON null"
    );
}

// =========================================================================
// (3) RIGHT JOIN -> exact row-set incl. null-fill for the unmatched right row
// =========================================================================

/// GIVEN customer 30 has no referencing order
/// WHEN a RIGHT JOIN runs
/// THEN the two matched orders plus a synthetic row for customer 30 appear.
/// The synthetic row's id is the customer primary key and it lacks the
/// base-only `title` column.
#[test]
fn test_right_join_exact_with_unmatched_right() {
    let (_dir, db) = create_test_db();
    setup_orders_customers(&db);

    let sql = "SELECT * FROM orders \
               RIGHT JOIN customers ON orders.customer_id = customers.id \
               LIMIT 50";
    let results = execute_sql(&db, sql).expect("RIGHT fk JOIN");

    // Matched rows keep the base order id; the unmatched right row uses the
    // customer primary key (30).
    assert_eq!(result_ids(&results), HashSet::from([1, 2, 30]));
    assert_eq!(payload_str(row(&results, 30), "name"), Some("Cust-Z"));
    assert_eq!(
        payload_str(row(&results, 30), "title"),
        None,
        "unmatched RIGHT row must not carry the base-only title column"
    );
}

// =========================================================================
// (4) FULL OUTER JOIN -> exact union with null-fill on both sides
// =========================================================================

/// GIVEN one unmatched order (3) and one unmatched customer (30)
/// WHEN a FULL OUTER JOIN runs
/// THEN the union of matched pairs + both unmatched rows appears: matched {1,2},
/// unmatched-left {3} with null joined name, unmatched-right {30}.
#[test]
fn test_full_outer_join_exact_union() {
    let (_dir, db) = create_test_db();
    setup_orders_customers(&db);

    let sql = "SELECT * FROM orders \
               FULL JOIN customers ON orders.customer_id = customers.id \
               LIMIT 50";
    let results = execute_sql(&db, sql).expect("FULL fk JOIN");

    assert_eq!(result_ids(&results), HashSet::from([1, 2, 3, 30]));
    assert_eq!(payload_str(row(&results, 2), "name"), Some("Cust-Y"));
    // Unmatched left: own title kept, joined name null.
    assert!(field_is_json_null(row(&results, 3), "name"));
    assert_eq!(payload_str(row(&results, 3), "title"), Some("ord-c"));
    // Unmatched right: joined name present, no base title.
    assert_eq!(payload_str(row(&results, 30), "name"), Some("Cust-Z"));
    assert_eq!(payload_str(row(&results, 30), "title"), None);
}

// =========================================================================
// (5) 3-way chained JOIN -> exact merged row
// =========================================================================

/// Creates a 3-level fk chain: categories -> groups -> depts.
fn setup_three_way_chain(db: &Database) {
    execute_sql(
        db,
        "CREATE COLLECTION categories (dimension = 4, metric = 'cosine');",
    )
    .expect("CREATE categories");
    let categories = db
        .get_vector_collection("categories")
        .expect("get categories");
    categories
        .upsert(vec![Point::new(
            1,
            vec![1.0, 0.0, 0.0, 0.0],
            Some(json!({"cat_name": "Cat-1", "parent_fk": 100})),
        )])
        .expect("upsert categories");

    execute_sql(db, "CREATE METADATA COLLECTION groups;").expect("CREATE groups");
    db.get_metadata_collection("groups")
        .expect("get groups")
        .upsert(vec![Point::metadata_only(
            100,
            json!({"group_name": "Grp-100", "dept_fk": 500}),
        )])
        .expect("upsert groups");

    execute_sql(db, "CREATE METADATA COLLECTION depts;").expect("CREATE depts");
    db.get_metadata_collection("depts")
        .expect("get depts")
        .upsert(vec![Point::metadata_only(
            500,
            json!({"dept_name": "Dept-500"}),
        )])
        .expect("upsert depts");
}

/// GIVEN a categories -> groups -> depts foreign-key chain
/// WHEN a 3-way chained JOIN runs
/// THEN one row is produced carrying every collection's field, keyed by the
/// base category id.
#[test]
fn test_three_way_chained_join_exact_row() {
    let (_dir, db) = create_test_db();
    setup_three_way_chain(&db);

    let sql = "SELECT * FROM categories \
               JOIN groups ON categories.parent_fk = groups.id \
               JOIN depts ON groups.dept_fk = depts.id \
               LIMIT 50";
    let results = execute_sql(&db, sql).expect("3-way chained JOIN");

    assert_eq!(result_ids(&results), HashSet::from([1]));
    let r = row(&results, 1);
    assert_eq!(payload_str(r, "cat_name"), Some("Cat-1"));
    assert_eq!(payload_str(r, "group_name"), Some("Grp-100"));
    assert_eq!(payload_str(r, "dept_name"), Some("Dept-500"));
    assert_eq!(
        payload_f64(r, "dept_fk"),
        Some(500.0),
        "intermediate fk from the first join must survive into the merged row"
    );
}

// =========================================================================
// (6) Self-JOIN via alias -> exact rows
// =========================================================================

/// Creates a `staff` collection modelling an employee -> manager hierarchy.
/// staff 1 (Alice) and 2 (Bob) report to staff 3 (Carol); Carol has no manager.
fn setup_self_join_staff(db: &Database) {
    execute_sql(
        db,
        "CREATE COLLECTION staff (dimension = 4, metric = 'cosine');",
    )
    .expect("CREATE staff");
    db.get_vector_collection("staff")
        .expect("get staff")
        .upsert(vec![
            Point::new(
                1,
                vec![1.0, 0.0, 0.0, 0.0],
                Some(json!({"emp_name": "Alice", "manager_id": 3})),
            ),
            Point::new(
                2,
                vec![0.0, 1.0, 0.0, 0.0],
                Some(json!({"emp_name": "Bob", "manager_id": 3})),
            ),
            Point::new(
                3,
                vec![0.0, 0.0, 1.0, 0.0],
                Some(json!({"emp_name": "Carol", "manager_id": 0})),
            ),
        ])
        .expect("upsert staff");
}

/// GIVEN staff joined to itself via aliases on manager_id = id
/// WHEN an INNER self-JOIN runs
/// THEN only the two employees with a real manager appear, keyed by the
/// employee id, and the joined (manager) side overwrites the colliding
/// `emp_name` column per the engine's right-wins merge.
#[test]
fn test_self_join_exact_rows() {
    let (_dir, db) = create_test_db();
    setup_self_join_staff(&db);

    let sql = "SELECT * FROM staff AS e \
               JOIN staff AS m ON e.manager_id = m.id \
               LIMIT 50";
    let results = execute_sql(&db, sql).expect("self JOIN");

    // Employees 1 and 2 have manager 3; employee 3's manager_id (0) matches no id.
    assert_eq!(result_ids(&results), HashSet::from([1, 2]));
    // Right side (manager Carol) overwrites the colliding emp_name on merge.
    assert_eq!(payload_str(row(&results, 1), "emp_name"), Some("Carol"));
    assert_eq!(payload_str(row(&results, 2), "emp_name"), Some("Carol"));
}

// =========================================================================
// The joined row as SQL reads it: each side under its alias, the window
// after the join, aggregates over the joined rows
// =========================================================================

/// GIVEN orders joined to customers
/// WHEN the SELECT projects and orders by qualified columns
/// THEN each side reads under its alias, sorted after the join.
#[test]
fn qualified_columns_read_each_side_after_the_join() {
    let (_dir, db) = create_test_db();
    setup_orders_customers(&db);
    let rows = execute_sql(
        &db,
        "SELECT * FROM orders AS o JOIN customers AS c ON o.customer_id = c.id \
         ORDER BY c.name DESC",
    )
    .expect("JOIN ORDER BY a joined column");
    let names: Vec<&str> = rows
        .iter()
        .filter_map(|r| r.point.payload.as_ref()?.get("c")?.get("name")?.as_str())
        .collect();
    assert_eq!(names, vec!["Cust-Y", "Cust-X"], "{rows:?}");
}

/// GIVEN three orders, one whose customer does not exist
/// WHEN `LIMIT 2` asks for two INNER-joined rows
/// THEN two come back: the LIMIT windows the joined rows, not the base rows
///      the join then drops.
#[test]
fn the_limit_windows_the_joined_rows() {
    let (_dir, db) = create_test_db();
    setup_orders_customers(&db);
    let rows = execute_sql(
        &db,
        "SELECT * FROM orders JOIN customers ON orders.customer_id = customers.id \
         ORDER BY orders.id DESC LIMIT 2",
    )
    .expect("JOIN LIMIT");
    let ids: Vec<u64> = rows.iter().map(|r| r.point.id).collect();
    assert_eq!(ids, vec![2, 1], "order 3 has no customer");
}

/// GIVEN orders joined to customers
/// WHEN an aggregate without GROUP BY counts the joined rows
/// THEN it counts them, not the base collection's points.
#[test]
fn an_aggregate_counts_the_joined_rows() {
    let (_dir, db) = create_test_db();
    setup_orders_customers(&db);
    let query = velesdb_core::velesql::Parser::parse(
        "SELECT COUNT(*) AS n FROM orders JOIN customers ON orders.customer_id = customers.id",
    )
    .expect("parse");
    let result = db
        .execute_aggregate(&query, &std::collections::HashMap::new())
        .expect("aggregate over a JOIN");
    assert_eq!(result, json!({"n": 2}), "orders 1 and 2 have a customer");
}

/// GIVEN orders LEFT-joined to customers
/// WHEN the WHERE compares a joined-side column a matched row satisfies
/// THEN that row comes back: the predicate reads the joined row, not the
///      base row before the join.
#[test]
fn a_left_join_where_on_the_joined_side_keeps_the_matching_rows() {
    let (_dir, db) = create_test_db();
    setup_orders_customers(&db);
    let rows = execute_sql(
        &db,
        "SELECT * FROM orders LEFT JOIN customers ON orders.customer_id = customers.id \
         WHERE customers.name = 'Cust-X'",
    )
    .expect("LEFT JOIN WHERE on the joined side");
    assert_eq!(result_ids(&rows), HashSet::from([1]), "{rows:?}");
}

/// GIVEN a category joined to its group and that group's dept (two INNER
///       joins)
/// WHEN the WHERE ORs a predicate on each join side, or negates that OR
/// THEN the row comes back exactly when the condition holds on the joined
///      row: a predicate spanning two sides is evaluated after the joins.
#[test]
fn a_where_spanning_two_join_sides_is_evaluated_on_the_joined_row() {
    let (_dir, db) = create_test_db();
    setup_three_way_chain(&db);
    let ids = |predicate: &str| {
        let sql = format!(
            "SELECT * FROM categories \
             JOIN groups ON categories.parent_fk = groups.id \
             JOIN depts ON groups.dept_fk = depts.id WHERE {predicate}"
        );
        result_ids(&execute_sql(&db, &sql).unwrap_or_else(|e| panic!("{sql}: {e}")))
    };
    let neither = "groups.group_name = 'none' OR depts.dept_name = 'none'";
    let one = "groups.group_name = 'none' OR depts.dept_name = 'Dept-500'";
    assert_eq!(ids(neither), HashSet::new(), "{neither}");
    assert_eq!(ids(one), HashSet::from([1]), "{one}");
    assert_eq!(
        ids(&format!("NOT ({neither})")),
        HashSet::from([1]),
        "NOT ({neither})"
    );
}

/// GIVEN orders joined to customers
/// WHEN the WHERE keeps no joined row
/// THEN an aggregate without GROUP BY still reports each column over no row
///      (`COUNT(*)` 0), as an aggregate over a scan does.
#[test]
fn an_aggregate_over_no_joined_row_reports_its_empty_values() {
    let (_dir, db) = create_test_db();
    setup_orders_customers(&db);
    for predicate in ["customers.name = 'none'", "orders.title = 'ord-c'"] {
        let query = velesdb_core::velesql::Parser::parse(&format!(
            "SELECT COUNT(*) AS n FROM orders JOIN customers ON orders.customer_id = customers.id \
             WHERE {predicate}"
        ))
        .expect("parse");
        let result = db
            .execute_aggregate(&query, &std::collections::HashMap::new())
            .expect("aggregate over an empty JOIN");
        assert_eq!(result, json!({"n": 0}), "{predicate}");
    }
}

/// GIVEN customers whose payloads hold a nested object, an array, and a
///       field whose type varies from one customer to the next
/// WHEN orders join them
/// THEN every joined field reads, as the customer's own payload holds it
///      (the column-store copy core joined against dropped all three).
#[test]
fn every_joined_field_reads_whatever_its_shape() {
    let (_dir, db) = create_test_db();
    execute_sql(&db, "CREATE METADATA COLLECTION buyers;").expect("buyers");
    execute_sql(&db, "CREATE METADATA COLLECTION sales;").expect("sales");
    db.get_metadata_collection("buyers")
        .expect("buyers")
        .upsert(vec![
            Point::metadata_only(
                1,
                json!({"meta": {"tier": "gold"}, "tags": ["a"], "code": 7}),
            ),
            Point::metadata_only(
                2,
                json!({"meta": {"tier": "free"}, "tags": [], "code": "x"}),
            ),
        ])
        .expect("upsert buyers");
    db.get_metadata_collection("sales")
        .expect("sales")
        .upsert(vec![
            Point::metadata_only(10, json!({"buyer": 1})),
            Point::metadata_only(11, json!({"buyer": 2})),
        ])
        .expect("upsert sales");

    let rows = execute_sql(
        &db,
        "SELECT * FROM sales JOIN buyers ON sales.buyer = buyers.id \
         WHERE buyers.meta.tier = 'gold' LIMIT 10",
    )
    .expect("JOIN reading a nested joined field");
    assert_eq!(result_ids(&rows), HashSet::from([10]));
    let payload = rows[0].point.payload.as_ref().expect("payload");
    assert_eq!(payload["buyers"]["tags"], json!(["a"]));
    assert_eq!(payload["buyers"]["code"], json!(7));
}
