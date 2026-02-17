//! E2E integration tests for JOIN execution across collections.
//!
//! Validates that `Database::execute_query()` correctly wires
//! INNER JOIN, LEFT JOIN, chained JOINs, and error handling
//! through the full `VelesQL` pipeline.

use std::collections::HashMap;

use serde_json::json;

use velesdb_core::distance::DistanceMetric;
use velesdb_core::{Database, Point};

/// Creates a Database with "products" and "inventory" collections
/// simulating an e-commerce scenario.
fn setup_ecommerce_db() -> (Database, tempfile::TempDir) {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let db = Database::open(dir.path()).expect("open");

    // Products collection
    db.create_collection("products", 4, DistanceMetric::Cosine)
        .expect("create products");
    let products = db.get_collection("products").expect("get products");
    products
        .upsert(vec![
            Point::new(
                1,
                vec![1.0, 0.0, 0.0, 0.0],
                Some(json!({"id": 1, "sku": "SKU-001", "title": "Laptop Pro", "category": "electronics"})),
            ),
            Point::new(
                2,
                vec![0.0, 1.0, 0.0, 0.0],
                Some(json!({"id": 2, "sku": "SKU-002", "title": "Wireless Mouse", "category": "electronics"})),
            ),
            Point::new(
                3,
                vec![0.0, 0.0, 1.0, 0.0],
                Some(json!({"id": 3, "sku": "SKU-003", "title": "Standing Desk", "category": "furniture"})),
            ),
            Point::new(
                4,
                vec![0.0, 0.0, 0.0, 1.0],
                Some(json!({"id": 4, "sku": "SKU-004", "title": "USB Hub", "category": "electronics"})),
            ),
        ])
        .expect("upsert products");

    // Inventory collection (only some products have stock)
    db.create_collection("inventory", 4, DistanceMetric::Cosine)
        .expect("create inventory");
    let inventory = db.get_collection("inventory").expect("get inventory");
    inventory
        .upsert(vec![
            Point::new(
                1,
                vec![0.5, 0.5, 0.0, 0.0],
                Some(json!({"id": 1, "sku": "SKU-001", "stock": 42, "warehouse": "NYC"})),
            ),
            Point::new(
                2,
                vec![0.0, 0.5, 0.5, 0.0],
                Some(json!({"id": 2, "sku": "SKU-002", "stock": 0, "warehouse": "LA"})),
            ),
            Point::new(
                3,
                vec![0.5, 0.0, 0.5, 0.0],
                Some(json!({"id": 3, "sku": "SKU-003", "stock": 15, "warehouse": "NYC"})),
            ),
        ])
        .expect("upsert inventory");

    (db, dir)
}

/// Creates a Database with "documents" and "authors" collections
/// for RAG-style metadata enrichment.
fn setup_rag_db() -> (Database, tempfile::TempDir) {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let db = Database::open(dir.path()).expect("open");

    db.create_collection("documents", 4, DistanceMetric::Cosine)
        .expect("create documents");
    let docs = db.get_collection("documents").expect("get documents");
    docs.upsert(vec![
        Point::new(
            1,
            vec![1.0, 0.0, 0.0, 0.0],
            Some(json!({"id": 1, "author_id": 10, "title": "Vector Search Guide", "topic": "search"})),
        ),
        Point::new(
            2,
            vec![0.9, 0.1, 0.0, 0.0],
            Some(json!({"id": 2, "author_id": 10, "title": "HNSW Deep Dive", "topic": "search"})),
        ),
        Point::new(
            3,
            vec![0.0, 1.0, 0.0, 0.0],
            Some(json!({"id": 3, "author_id": 20, "title": "Graph Databases", "topic": "graph"})),
        ),
        Point::new(
            4,
            vec![0.0, 0.0, 1.0, 0.0],
            Some(json!({"id": 4, "author_id": 30, "title": "Cooking with AI", "topic": "misc"})),
        ),
    ])
    .expect("upsert documents");

    db.create_collection("authors", 4, DistanceMetric::Cosine)
        .expect("create authors");
    let authors = db.get_collection("authors").expect("get authors");
    authors
        .upsert(vec![
            Point::new(
                10,
                vec![0.5, 0.5, 0.0, 0.0],
                Some(json!({"id": 10, "name": "Alice Chen", "affiliation": "MIT"})),
            ),
            Point::new(
                20,
                vec![0.0, 0.5, 0.5, 0.0],
                Some(json!({"id": 20, "name": "Bob Smith", "affiliation": "Stanford"})),
            ),
        ])
        .expect("upsert authors");

    (db, dir)
}

// ========== E-Commerce JOIN Scenarios ==========

#[test]
fn test_e2e_ecommerce_inner_join_products_inventory() {
    let (db, _dir) = setup_ecommerce_db();

    let query = velesdb_core::velesql::Parser::parse(
        "SELECT * FROM products JOIN inventory ON inventory.id = products.id LIMIT 10",
    )
    .expect("parse");
    let params = HashMap::new();
    let results = db.execute_query(&query, &params).expect("execute");

    // Products {1,2,3,4} ∩ Inventory {1,2,3} → INNER JOIN yields {1,2,3}
    assert_eq!(results.len(), 3, "INNER JOIN should return 3 matching rows");

    let ids: std::collections::HashSet<u64> = results.iter().map(|r| r.point.id).collect();
    assert!(ids.contains(&1));
    assert!(ids.contains(&2));
    assert!(ids.contains(&3));
    assert!(!ids.contains(&4), "Product 4 has no inventory match");

    // Verify merged payload contains inventory data
    let product1 = results.iter().find(|r| r.point.id == 1).unwrap();
    let payload = product1.point.payload.as_ref().unwrap();
    assert!(
        payload.get("stock").is_some() || payload.get("warehouse").is_some(),
        "Joined row should have inventory fields"
    );
}

#[test]
fn test_e2e_ecommerce_vector_search_plus_join() {
    let (db, _dir) = setup_ecommerce_db();

    // Vector search for electronics + JOIN with inventory
    let query = velesdb_core::velesql::Parser::parse(
        "SELECT * FROM products JOIN inventory ON inventory.id = products.id \
         WHERE vector NEAR $query_vec LIMIT 5",
    )
    .expect("parse");
    let mut params = HashMap::new();
    params.insert("query_vec".to_string(), json!([1.0, 0.0, 0.0, 0.0]));

    let results = db.execute_query(&query, &params).expect("execute");

    // Vector search returns nearest neighbors, then JOIN filters to those in inventory
    assert!(!results.is_empty(), "NEAR + JOIN should return results");

    // All results should have inventory data merged
    for result in &results {
        let payload = result.point.payload.as_ref().unwrap();
        assert!(
            payload.get("stock").is_some() || payload.get("warehouse").is_some(),
            "NEAR + JOIN result should have inventory data, id={}",
            result.point.id
        );
    }
}

// ========== RAG with Metadata JOIN Scenarios ==========

#[test]
fn test_e2e_rag_left_join_documents_authors() {
    let (db, _dir) = setup_rag_db();

    let query = velesdb_core::velesql::Parser::parse(
        "SELECT * FROM documents LEFT JOIN authors ON authors.id = documents.author_id LIMIT 10",
    )
    .expect("parse");
    let params = HashMap::new();
    let results = db.execute_query(&query, &params).expect("execute");

    // LEFT JOIN keeps all 4 documents
    assert_eq!(
        results.len(),
        4,
        "LEFT JOIN should keep all left rows (4 documents)"
    );

    // Doc 4 (author_id=30) has no matching author — should still be present
    let doc4 = results.iter().find(|r| r.point.id == 4);
    assert!(
        doc4.is_some(),
        "Doc with no author match should still appear in LEFT JOIN"
    );

    // Doc 1 (author_id=10) should have merged author data
    let doc1 = results.iter().find(|r| r.point.id == 1).unwrap();
    let payload = doc1.point.payload.as_ref().unwrap();
    assert!(
        payload.get("name").is_some(),
        "Matched doc should have author 'name' field"
    );
}

// ========== Chained JOIN Scenarios ==========

#[test]
fn test_e2e_chained_two_joins() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let db = Database::open(dir.path()).expect("open");

    // Create 3 collections for a chained JOIN
    db.create_collection("orders", 4, DistanceMetric::Cosine)
        .expect("create");
    db.create_collection("customers", 4, DistanceMetric::Cosine)
        .expect("create");
    db.create_collection("products", 4, DistanceMetric::Cosine)
        .expect("create");

    let orders = db.get_collection("orders").expect("get");
    orders
        .upsert(vec![
            Point::new(
                1,
                vec![1.0, 0.0, 0.0, 0.0],
                Some(json!({"id": 1, "customer_id": 100, "product_id": 200, "amount": 99.99})),
            ),
            Point::new(
                2,
                vec![0.0, 1.0, 0.0, 0.0],
                Some(json!({"id": 2, "customer_id": 100, "product_id": 201, "amount": 149.99})),
            ),
        ])
        .expect("upsert");

    let customers = db.get_collection("customers").expect("get");
    customers
        .upsert(vec![Point::new(
            100,
            vec![0.5, 0.5, 0.0, 0.0],
            Some(json!({"id": 100, "name": "Alice", "tier": "gold"})),
        )])
        .expect("upsert");

    let products = db.get_collection("products").expect("get");
    products
        .upsert(vec![
            Point::new(
                200,
                vec![0.0, 0.0, 1.0, 0.0],
                Some(json!({"id": 200, "product_name": "Laptop", "brand": "TechCo"})),
            ),
            Point::new(
                201,
                vec![0.0, 0.0, 0.0, 1.0],
                Some(json!({"id": 201, "product_name": "Keyboard", "brand": "TypeFast"})),
            ),
        ])
        .expect("upsert");

    // Chained JOIN: orders → customers → products
    let query = velesdb_core::velesql::Parser::parse(
        "SELECT * FROM orders \
         JOIN customers ON customers.id = orders.customer_id \
         JOIN products ON products.id = orders.product_id \
         LIMIT 10",
    )
    .expect("parse");
    let params = HashMap::new();
    let results = db.execute_query(&query, &params).expect("execute");

    // Both orders match customer 100 and their respective products
    assert_eq!(
        results.len(),
        2,
        "Chained JOIN should return 2 matching rows"
    );

    // Verify data from all 3 collections merged
    for result in &results {
        let payload = result.point.payload.as_ref().unwrap();
        assert!(
            payload.get("name").is_some(),
            "Should have 'name' from customers, id={}",
            result.point.id
        );
        assert!(
            payload.get("product_name").is_some(),
            "Should have 'product_name' from products, id={}",
            result.point.id
        );
    }
}

// ========== JOIN + ORDER BY + LIMIT ==========

#[test]
fn test_e2e_join_order_by_limit() {
    let (db, _dir) = setup_rag_db();

    let query = velesdb_core::velesql::Parser::parse(
        "SELECT * FROM documents JOIN authors ON authors.id = documents.author_id \
         ORDER BY title ASC LIMIT 2",
    )
    .expect("parse");
    let params = HashMap::new();
    let results = db.execute_query(&query, &params).expect("execute");

    // INNER JOIN matches on payload field author_id = authors.id (point ID)
    // LIMIT 2 caps the final result count
    assert!(
        results.len() <= 2,
        "LIMIT 2 should cap results, got {}",
        results.len()
    );

    // Verify ORDER BY title ASC is respected (sorted alphabetically)
    if results.len() == 2 {
        let titles: Vec<&str> = results
            .iter()
            .filter_map(|r| {
                r.point
                    .payload
                    .as_ref()
                    .and_then(|p| p.get("title"))
                    .and_then(|t| t.as_str())
            })
            .collect();
        // Verify alphabetical ascending order
        assert!(
            titles.windows(2).all(|w| w[0] <= w[1]),
            "Results should be sorted by title ASC, got {titles:?}"
        );
    }
}

// ========== Error Handling ==========

#[test]
fn test_e2e_join_table_not_found() {
    let (db, _dir) = setup_rag_db();

    let query = velesdb_core::velesql::Parser::parse(
        "SELECT * FROM documents JOIN nonexistent_table ON nonexistent_table.id = documents.id LIMIT 10",
    )
    .expect("parse");
    let params = HashMap::new();
    let result = db.execute_query(&query, &params);

    assert!(result.is_err(), "JOIN on non-existent table should error");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("nonexistent_table"),
        "Error should mention missing collection name: {err_msg}"
    );
}
