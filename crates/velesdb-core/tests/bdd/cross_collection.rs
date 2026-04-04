//! BDD tests for cross-collection queries (Issue #495).
//!
//! Validates:
//! - JOIN between VectorCollection and MetadataCollection
//! - JOIN between VectorCollection and MetadataCollection with vector search
//! - MATCH queries routed through Database::execute_query
//! - Three collection types coexisting and independently queryable

use std::collections::HashMap;

use serde_json::json;
use velesdb_core::{Database, Point};

use super::helpers::{
    create_test_db, execute_sql, execute_sql_with_params, payload_str, vector_param,
};

// =========================================================================
// Helpers
// =========================================================================

/// Creates a VectorCollection `products` with 5 products and a
/// MetadataCollection `inventory` with matching inventory rows.
fn setup_cross_type_collections(db: &Database) {
    execute_sql(
        db,
        "CREATE COLLECTION products (dimension = 4, metric = 'cosine');",
    )
    .expect("test: CREATE products");

    let products = db
        .get_vector_collection("products")
        .expect("test: get products");
    products
        .upsert(vec![
            Point::new(
                1,
                vec![1.0, 0.0, 0.0, 0.0],
                Some(json!({"name": "Headphones", "category": "audio"})),
            ),
            Point::new(
                2,
                vec![0.0, 1.0, 0.0, 0.0],
                Some(json!({"name": "Keyboard", "category": "input"})),
            ),
            Point::new(
                3,
                vec![0.0, 0.0, 1.0, 0.0],
                Some(json!({"name": "Monitor", "category": "display"})),
            ),
            Point::new(
                4,
                vec![0.0, 0.0, 0.0, 1.0],
                Some(json!({"name": "Mouse", "category": "input"})),
            ),
            Point::new(
                5,
                vec![0.7, 0.7, 0.0, 0.0],
                Some(json!({"name": "Speakers", "category": "audio"})),
            ),
        ])
        .expect("test: upsert products");

    execute_sql(db, "CREATE METADATA COLLECTION inventory;").expect("test: CREATE inventory");

    let inventory = db
        .get_metadata_collection("inventory")
        .expect("test: get inventory");
    inventory
        .upsert(vec![
            Point::metadata_only(1, json!({"product_id": 1, "price": 99.99, "stock": 50})),
            Point::metadata_only(2, json!({"product_id": 2, "price": 149.99, "stock": 0})),
            Point::metadata_only(3, json!({"product_id": 3, "price": 399.99, "stock": 12})),
            Point::metadata_only(4, json!({"product_id": 4, "price": 29.99, "stock": 200})),
            Point::metadata_only(5, json!({"product_id": 5, "price": 79.99, "stock": 30})),
        ])
        .expect("test: upsert inventory");
}

/// Creates a GraphCollection `social` with nodes and edges for MATCH tests.
fn setup_graph_collection(db: &Database) {
    execute_sql(
        db,
        "CREATE GRAPH COLLECTION social (dimension = 4, metric = 'cosine') SCHEMALESS;",
    )
    .expect("test: CREATE social graph");

    let gc = db.get_graph_collection("social").expect("test: get social");

    gc.upsert_node_payload(10, &json!({"_labels": ["Person"], "name": "Alice"}))
        .expect("test: node Alice");
    gc.upsert_node_payload(20, &json!({"_labels": ["Person"], "name": "Bob"}))
        .expect("test: node Bob");
    gc.upsert_node_payload(30, &json!({"_labels": ["Person"], "name": "Charlie"}))
        .expect("test: node Charlie");

    use velesdb_core::GraphEdge;
    gc.add_edge(GraphEdge::new(1, 10, 20, "KNOWS").expect("test: create edge"))
        .expect("test: add edge Alice->Bob");
    gc.add_edge(GraphEdge::new(2, 20, 30, "KNOWS").expect("test: create edge"))
        .expect("test: add edge Bob->Charlie");
    gc.add_edge(GraphEdge::new(3, 10, 30, "FOLLOWS").expect("test: create edge"))
        .expect("test: add edge Alice->Charlie");
}

// =========================================================================
// Scenario 1: VectorCollection JOIN MetadataCollection
// =========================================================================

/// GIVEN a VectorCollection `products` and a MetadataCollection `inventory`
/// WHEN a JOIN query combines both collections
/// THEN results contain fields from both collections.
#[test]
fn test_join_vector_and_metadata_collections() {
    let (_dir, db) = create_test_db();
    setup_cross_type_collections(&db);

    // VelesQL JOIN syntax: FROM table JOIN table ON condition
    let sql = "SELECT * FROM products \
               JOIN inventory ON products.id = inventory.id \
               LIMIT 10";

    let results = execute_sql(&db, sql).expect("test: cross-type JOIN should succeed");

    assert!(
        !results.is_empty(),
        "JOIN between VectorCollection and MetadataCollection should return results"
    );
}

// =========================================================================
// Scenario 2: VectorCollection JOIN MetadataCollection + vector search
// =========================================================================

/// GIVEN a VectorCollection `products` and a MetadataCollection `inventory`
/// WHEN a JOIN query includes vector NEAR search
/// THEN results are ordered by vector similarity AND enriched with inventory data.
#[test]
fn test_join_vector_metadata_with_near_search() {
    let (_dir, db) = create_test_db();
    setup_cross_type_collections(&db);

    let sql = "SELECT * FROM products \
               JOIN inventory ON products.id = inventory.id \
               WHERE vector NEAR $v \
               LIMIT 5";

    let params = vector_param(&[1.0, 0.0, 0.0, 0.0]);
    let results =
        execute_sql_with_params(&db, sql, &params).expect("test: JOIN + NEAR should succeed");

    assert!(!results.is_empty(), "JOIN + NEAR should return results");

    // The closest vector to [1,0,0,0] is product 1 (Headphones)
    let first = &results[0];
    assert_eq!(
        payload_str(first, "name"),
        Some("Headphones"),
        "First result should be Headphones (closest to query vector)"
    );
}

// =========================================================================
// Scenario 3: MATCH query via Database::execute_query with _collection param
// =========================================================================

/// GIVEN a GraphCollection `social` with nodes and edges
/// WHEN a MATCH query is executed via Database::execute_query with `_collection` param
/// THEN the query succeeds and returns traversal results.
#[test]
fn test_match_via_database_with_collection_param() {
    let (_dir, db) = create_test_db();
    setup_graph_collection(&db);

    let gc = db.get_graph_collection("social").expect("test: get social");
    gc.flush().expect("test: flush social");

    let sql = "MATCH (a:Person)-[:KNOWS]->(b:Person) RETURN a, b LIMIT 10";

    let mut params = HashMap::new();
    params.insert("_collection".to_string(), serde_json::json!("social"));

    let results = execute_sql_with_params(&db, sql, &params)
        .expect("test: MATCH via Database with _collection param should succeed");

    assert!(
        !results.is_empty(),
        "MATCH via Database should return results for Alice->Bob, Bob->Charlie"
    );
}

// =========================================================================
// Scenario 4: MATCH without collection param returns clear error
// =========================================================================

/// GIVEN a GraphCollection `social`
/// WHEN a MATCH query is sent without FROM or _collection
/// THEN a clear error guides the user.
#[test]
fn test_match_without_collection_returns_guidance_error() {
    let (_dir, db) = create_test_db();
    setup_graph_collection(&db);

    let sql = "MATCH (a:Person)-[:KNOWS]->(b:Person) RETURN a, b LIMIT 10";
    let err = execute_sql(&db, sql).expect_err("test: MATCH without collection should error");

    let msg = err.to_string();
    assert!(
        msg.contains("target collection"),
        "Error should guide user to specify collection, got: {msg}"
    );
}

// =========================================================================
// Scenario 5: Three collection types coexist and are queryable
// =========================================================================

/// GIVEN all 3 collection types created in the same database
/// WHEN each is queried independently
/// THEN all queries succeed without interference.
#[test]
fn test_three_collection_types_independent_queries() {
    let (_dir, db) = create_test_db();
    setup_cross_type_collections(&db);
    setup_graph_collection(&db);

    // Query vector collection
    let v_results = execute_sql(&db, "SELECT * FROM products LIMIT 5")
        .expect("test: SELECT from VectorCollection");
    assert_eq!(v_results.len(), 5, "products should have 5 rows");

    // Query metadata collection
    let m_results = execute_sql(&db, "SELECT * FROM inventory LIMIT 5")
        .expect("test: SELECT from MetadataCollection");
    assert_eq!(m_results.len(), 5, "inventory should have 5 rows");

    // Query graph collection
    let g_results = execute_sql(&db, "SELECT * FROM social LIMIT 5")
        .expect("test: SELECT from GraphCollection");
    assert!(!g_results.is_empty(), "social should have nodes");
}
