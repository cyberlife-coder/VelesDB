//! BDD tests: golden JOIN row-sets across all four join types plus chained and
//! self joins.
//!
//! The sibling `cross_collection_join_optimization.rs` only pins INNER/LEFT
//! cardinalities; this module asserts the *exact* merged row-set (ids + field
//! values + null-fill) for INNER / LEFT / RIGHT / FULL, a 3-way chained JOIN,
//! and a self-JOIN.
//!
//! Routing notes verified against the engine source:
//! - `Database::execute_single_join` (database/query_join.rs) takes the lookup
//!   path only when both join sides reference `id` AND no filter is pushed; that
//!   path (`execute_lookup_join`) appends unmatched rows for LEFT only. RIGHT and
//!   FULL therefore exercise the ColumnStore path in `execute_join`
//!   (collection/search/query/join.rs:134), which performs the documented
//!   null-fill on both sides.
//! - To force the ColumnStore path the base side is joined on a non-`id`
//!   foreign-key column (`base.fk = joined.id`); the joined side stays on `id`,
//!   which is the synthesized ColumnStore primary key
//!   (`build_join_column_store` -> `with_primary_key(.., "id")`).
//! - Unmatched LEFT rows carry joined columns as JSON `null`
//!   (`build_null_row_data`); unmatched RIGHT rows are synthetic points whose
//!   `id` is the right-side primary key and which lack base-only columns.

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
