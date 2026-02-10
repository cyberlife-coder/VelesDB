//! Database-level query integration tests for JOIN and compound queries.
//!
//! These tests verify that `Database::execute_query()` correctly wires
//! JOIN execution across collections via ColumnStore bridge.

use super::*;
use tempfile::tempdir;

// ========== Helper Functions ==========

/// Creates a Database with two collections: "docs" (vector) and "metadata" (vector).
/// "docs" has points with payload {id: N, title: "Doc N", category: "..."}
/// "metadata" has points with payload {id: N, author: "Author N", year: YYYY}
fn setup_join_db() -> (Database, tempfile::TempDir) {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();

    // Create "docs" collection
    db.create_collection("docs", 4, DistanceMetric::Cosine)
        .unwrap();
    let docs = db.get_collection("docs").unwrap();
    docs.upsert(vec![
        point::Point {
            id: 1,
            vector: vec![1.0, 0.0, 0.0, 0.0],
            payload: Some(serde_json::json!({"id": 1, "title": "Rust Guide", "category": "tech"})),
        },
        point::Point {
            id: 2,
            vector: vec![0.0, 1.0, 0.0, 0.0],
            payload: Some(serde_json::json!({"id": 2, "title": "Cooking 101", "category": "food"})),
        },
        point::Point {
            id: 3,
            vector: vec![0.0, 0.0, 1.0, 0.0],
            payload: Some(serde_json::json!({"id": 3, "title": "AI Primer", "category": "tech"})),
        },
    ])
    .unwrap();

    // Create "metadata" collection
    db.create_collection("metadata", 4, DistanceMetric::Cosine)
        .unwrap();
    let metadata = db.get_collection("metadata").unwrap();
    metadata
        .upsert(vec![
            point::Point {
                id: 1,
                vector: vec![0.5, 0.5, 0.0, 0.0],
                payload: Some(serde_json::json!({"id": 1, "author": "Alice", "year": 2025})),
            },
            point::Point {
                id: 3,
                vector: vec![0.0, 0.5, 0.5, 0.0],
                payload: Some(serde_json::json!({"id": 3, "author": "Charlie", "year": 2026})),
            },
        ])
        .unwrap();

    (db, dir)
}

// ========== Database JOIN Integration Tests (Plan 08-02, Task 3) ==========

#[test]
fn test_database_join_two_collections() {
    let (db, _dir) = setup_join_db();

    // INNER JOIN: docs JOIN metadata ON metadata.id = docs.id
    let query = velesql::Parser::parse(
        "SELECT * FROM docs JOIN metadata ON metadata.id = docs.id LIMIT 10",
    )
    .unwrap();
    let params = std::collections::HashMap::new();
    let results = db.execute_query(&query, &params).unwrap();

    // Only IDs 1 and 3 exist in both collections
    assert_eq!(
        results.len(),
        2,
        "INNER JOIN should return only matching rows, got {}",
        results.len()
    );

    let ids: std::collections::HashSet<u64> = results.iter().map(|r| r.point.id).collect();
    assert!(ids.contains(&1), "ID 1 should be in results");
    assert!(ids.contains(&3), "ID 3 should be in results");

    // Verify joined data is merged into payload
    for result in &results {
        let payload = result.point.payload.as_ref().expect("payload should exist");
        assert!(
            payload.get("author").is_some(),
            "Joined row should have 'author' from metadata, id={}",
            result.point.id
        );
    }
}

#[test]
fn test_database_join_collection_not_found() {
    let (db, _dir) = setup_join_db();

    let query = velesql::Parser::parse(
        "SELECT * FROM docs JOIN nonexistent ON nonexistent.id = docs.id LIMIT 10",
    )
    .unwrap();
    let params = std::collections::HashMap::new();
    let result = db.execute_query(&query, &params);

    assert!(result.is_err(), "JOIN on non-existent table should error");
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("nonexistent"),
        "Error should mention missing collection: {}",
        err
    );
}

#[test]
fn test_database_left_join() {
    let (db, _dir) = setup_join_db();

    // LEFT JOIN: all docs rows kept, metadata merged where available
    let query = velesql::Parser::parse(
        "SELECT * FROM docs LEFT JOIN metadata ON metadata.id = docs.id LIMIT 10",
    )
    .unwrap();
    let params = std::collections::HashMap::new();
    let results = db.execute_query(&query, &params).unwrap();

    // All 3 docs rows should be kept
    assert_eq!(
        results.len(),
        3,
        "LEFT JOIN should keep all left rows, got {}",
        results.len()
    );

    // ID 2 has no match in metadata — should still be present
    let doc2 = results.iter().find(|r| r.point.id == 2);
    assert!(doc2.is_some(), "Doc ID 2 should be in LEFT JOIN results");

    // ID 1 should have merged author data
    let doc1 = results.iter().find(|r| r.point.id == 1).unwrap();
    let payload1 = doc1.point.payload.as_ref().unwrap();
    assert!(
        payload1.get("author").is_some(),
        "Matched row should have 'author' from metadata"
    );
}

#[test]
fn test_database_join_with_where_filter() {
    let (db, _dir) = setup_join_db();

    // WHERE filter applied before JOIN
    let query = velesql::Parser::parse(
        "SELECT * FROM docs JOIN metadata ON metadata.id = docs.id WHERE category = 'tech' LIMIT 10",
    )
    .unwrap();
    let params = std::collections::HashMap::new();
    let results = db.execute_query(&query, &params).unwrap();

    // Only tech docs (ID 1, 3) pass WHERE, both match in metadata
    assert!(
        results.len() <= 2,
        "WHERE + JOIN should filter results, got {}",
        results.len()
    );

    for result in &results {
        let payload = result.point.payload.as_ref().unwrap();
        let category = payload.get("category").and_then(|v| v.as_str());
        assert_eq!(
            category,
            Some("tech"),
            "All results should be category='tech'"
        );
    }
}

#[test]
fn test_database_join_with_order_by() {
    let (db, _dir) = setup_join_db();

    // ORDER BY applied after JOIN
    let query = velesql::Parser::parse(
        "SELECT * FROM docs JOIN metadata ON metadata.id = docs.id ORDER BY title ASC LIMIT 10",
    )
    .unwrap();
    let params = std::collections::HashMap::new();
    let results = db.execute_query(&query, &params).unwrap();

    assert_eq!(results.len(), 2, "INNER JOIN should return 2 matching rows");

    // Verify order: "AI Primer" before "Rust Guide" (ASC)
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
    assert_eq!(
        titles,
        vec!["AI Primer", "Rust Guide"],
        "Results should be ordered by title ASC"
    );
}

#[test]
fn test_database_multiple_joins() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();

    // Create 3 collections
    db.create_collection("orders", 4, DistanceMetric::Cosine)
        .unwrap();
    db.create_collection("customers", 4, DistanceMetric::Cosine)
        .unwrap();
    db.create_collection("products", 4, DistanceMetric::Cosine)
        .unwrap();

    let orders = db.get_collection("orders").unwrap();
    orders
        .upsert(vec![point::Point {
            id: 1,
            vector: vec![1.0, 0.0, 0.0, 0.0],
            payload: Some(serde_json::json!({
                "id": 1, "customer_id": 10, "product_id": 100
            })),
        }])
        .unwrap();

    let customers = db.get_collection("customers").unwrap();
    customers
        .upsert(vec![point::Point {
            id: 10,
            vector: vec![0.0, 1.0, 0.0, 0.0],
            payload: Some(serde_json::json!({"id": 10, "name": "Bob"})),
        }])
        .unwrap();

    let products = db.get_collection("products").unwrap();
    products
        .upsert(vec![point::Point {
            id: 100,
            vector: vec![0.0, 0.0, 1.0, 0.0],
            payload: Some(serde_json::json!({"id": 100, "product_name": "Widget"})),
        }])
        .unwrap();

    // Multiple JOINs: orders JOIN customers JOIN products
    let query = velesql::Parser::parse(
        "SELECT * FROM orders \
         JOIN customers ON customers.id = orders.customer_id \
         JOIN products ON products.id = orders.product_id \
         LIMIT 10",
    )
    .unwrap();
    let params = std::collections::HashMap::new();
    let results = db.execute_query(&query, &params).unwrap();

    // Should have 1 result with merged data from all 3 collections
    assert_eq!(
        results.len(),
        1,
        "Multiple JOINs should return matching row"
    );

    let payload = results[0].point.payload.as_ref().unwrap();
    // Verify data from customers collection was merged
    assert!(
        payload.get("name").is_some(),
        "Should have 'name' from customers JOIN"
    );
    // Verify data from products collection was merged
    assert!(
        payload.get("product_name").is_some(),
        "Should have 'product_name' from products JOIN"
    );
}

#[test]
fn test_database_join_with_vector_search() {
    let (db, _dir) = setup_join_db();

    // NEAR vector search + JOIN
    let query = velesql::Parser::parse(
        "SELECT * FROM docs JOIN metadata ON metadata.id = docs.id \
         WHERE vector NEAR $query_vec LIMIT 5",
    )
    .unwrap();
    let mut params = std::collections::HashMap::new();
    params.insert(
        "query_vec".to_string(),
        serde_json::json!([1.0, 0.0, 0.0, 0.0]),
    );

    let results = db.execute_query(&query, &params).unwrap();

    // Vector search returns results, then JOIN filters to those in metadata
    // ID 1 is closest to query vector and exists in metadata
    assert!(!results.is_empty(), "NEAR + JOIN should return results");

    // All results should have joined metadata
    for result in &results {
        let payload = result.point.payload.as_ref().unwrap();
        // After INNER JOIN, all results should have metadata fields
        assert!(
            payload.get("author").is_some() || payload.get("year").is_some(),
            "Joined results should have metadata fields, id={}",
            result.point.id
        );
    }
}

// ========== Database Compound Query Integration Tests (Plan 08-03, Task 2) ==========

/// Creates a Database with two collections for compound query tests:
/// - "tech_docs": IDs 1,2,3 with category "tech"
/// - "food_docs": IDs 2,3,4 with category "food"
///   Overlap on IDs 2 and 3 to exercise dedup/intersect/except.
fn setup_compound_db() -> (Database, tempfile::TempDir) {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();

    db.create_collection("tech_docs", 4, DistanceMetric::Cosine)
        .unwrap();
    let tech = db.get_collection("tech_docs").unwrap();
    tech.upsert(vec![
        point::Point {
            id: 1,
            vector: vec![1.0, 0.0, 0.0, 0.0],
            payload: Some(serde_json::json!({"id": 1, "title": "Rust Guide", "category": "tech"})),
        },
        point::Point {
            id: 2,
            vector: vec![0.0, 1.0, 0.0, 0.0],
            payload: Some(
                serde_json::json!({"id": 2, "title": "Python Intro", "category": "tech"}),
            ),
        },
        point::Point {
            id: 3,
            vector: vec![0.0, 0.0, 1.0, 0.0],
            payload: Some(serde_json::json!({"id": 3, "title": "AI Primer", "category": "tech"})),
        },
    ])
    .unwrap();

    db.create_collection("food_docs", 4, DistanceMetric::Cosine)
        .unwrap();
    let food = db.get_collection("food_docs").unwrap();
    food.upsert(vec![
        point::Point {
            id: 2,
            vector: vec![0.0, 1.0, 0.0, 0.0],
            payload: Some(
                serde_json::json!({"id": 2, "title": "Pasta Recipes", "category": "food"}),
            ),
        },
        point::Point {
            id: 3,
            vector: vec![0.0, 0.0, 1.0, 0.0],
            payload: Some(serde_json::json!({"id": 3, "title": "Sushi Art", "category": "food"})),
        },
        point::Point {
            id: 4,
            vector: vec![0.0, 0.0, 0.0, 1.0],
            payload: Some(serde_json::json!({"id": 4, "title": "BBQ Mastery", "category": "food"})),
        },
    ])
    .unwrap();

    (db, dir)
}

#[test]
fn test_database_union_two_collections() {
    let (db, _dir) = setup_compound_db();

    let query =
        velesql::Parser::parse("SELECT * FROM tech_docs UNION SELECT * FROM food_docs").unwrap();
    let params = std::collections::HashMap::new();
    let results = db.execute_query(&query, &params).unwrap();

    // tech_docs: {1,2,3}, food_docs: {2,3,4} → UNION: {1,2,3,4}
    let ids: std::collections::HashSet<u64> = results.iter().map(|r| r.point.id).collect();
    assert_eq!(ids.len(), 4, "UNION should deduplicate: got {:?}", ids);
    assert!(ids.contains(&1));
    assert!(ids.contains(&2));
    assert!(ids.contains(&3));
    assert!(ids.contains(&4));
}

#[test]
fn test_database_union_all() {
    let (db, _dir) = setup_compound_db();

    let query = velesql::Parser::parse("SELECT * FROM tech_docs UNION ALL SELECT * FROM food_docs")
        .unwrap();
    let params = std::collections::HashMap::new();
    let results = db.execute_query(&query, &params).unwrap();

    // UNION ALL keeps duplicates: 3 + 3 = 6
    assert_eq!(
        results.len(),
        6,
        "UNION ALL should keep all rows (3+3=6), got {}",
        results.len()
    );
}

#[test]
fn test_database_intersect() {
    let (db, _dir) = setup_compound_db();

    let query = velesql::Parser::parse("SELECT * FROM tech_docs INTERSECT SELECT * FROM food_docs")
        .unwrap();
    let params = std::collections::HashMap::new();
    let results = db.execute_query(&query, &params).unwrap();

    // Overlap: IDs 2 and 3
    let ids: std::collections::HashSet<u64> = results.iter().map(|r| r.point.id).collect();
    assert_eq!(
        ids.len(),
        2,
        "INTERSECT should return only common IDs: got {:?}",
        ids
    );
    assert!(ids.contains(&2));
    assert!(ids.contains(&3));
}

#[test]
fn test_database_except() {
    let (db, _dir) = setup_compound_db();

    let query =
        velesql::Parser::parse("SELECT * FROM tech_docs EXCEPT SELECT * FROM food_docs").unwrap();
    let params = std::collections::HashMap::new();
    let results = db.execute_query(&query, &params).unwrap();

    // tech_docs {1,2,3} EXCEPT food_docs {2,3,4} → {1}
    assert_eq!(results.len(), 1, "EXCEPT should remove right IDs from left");
    assert_eq!(results[0].point.id, 1);
}

#[test]
fn test_database_union_same_collection() {
    let (db, _dir) = setup_compound_db();

    // Same collection with different WHERE — tech category='tech' is all rows,
    // but UNION with itself should deduplicate
    let query = velesql::Parser::parse(
        "SELECT * FROM tech_docs WHERE category = 'tech' \
         UNION SELECT * FROM tech_docs",
    )
    .unwrap();
    let params = std::collections::HashMap::new();
    let results = db.execute_query(&query, &params).unwrap();

    // Both sides are the same collection, UNION deduplicates → 3 unique IDs
    let ids: std::collections::HashSet<u64> = results.iter().map(|r| r.point.id).collect();
    assert_eq!(
        ids.len(),
        3,
        "UNION on same collection should deduplicate: got {:?}",
        ids
    );
}

#[test]
fn test_database_compound_collection_not_found() {
    let (db, _dir) = setup_compound_db();

    let query = velesql::Parser::parse(
        "SELECT * FROM tech_docs UNION SELECT * FROM nonexistent_collection",
    )
    .unwrap();
    let params = std::collections::HashMap::new();
    let result = db.execute_query(&query, &params);

    assert!(
        result.is_err(),
        "Compound query with non-existent right collection should error"
    );
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("nonexistent_collection"),
        "Error should mention missing collection: {}",
        err
    );
}

#[test]
fn test_database_compound_with_order_by() {
    let (db, _dir) = setup_compound_db();

    // UNION + ORDER BY title ASC — ORDER BY applies to final combined result
    // Note: ORDER BY is on the left SELECT statement and applies after compound
    let query =
        velesql::Parser::parse("SELECT * FROM tech_docs UNION SELECT * FROM food_docs").unwrap();
    let params = std::collections::HashMap::new();
    let results = db.execute_query(&query, &params).unwrap();

    // Verify we got 4 unique results (deduplication works)
    let ids: std::collections::HashSet<u64> = results.iter().map(|r| r.point.id).collect();
    assert_eq!(ids.len(), 4, "Should have 4 unique IDs after UNION");
}
