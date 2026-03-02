use super::*;
use crate::collection::graph::GraphEdge;
use crate::point::Point;
use crate::velesql::Parser;
use tempfile::tempdir;

#[test]
fn test_database_open() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    assert!(db.list_collections().is_empty());
}

#[test]
fn test_create_collection() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();

    db.create_collection("test", 768, DistanceMetric::Cosine)
        .unwrap();

    assert_eq!(db.list_collections(), vec!["test"]);
}

#[test]
fn test_duplicate_collection_error() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();

    db.create_collection("test", 768, DistanceMetric::Cosine)
        .unwrap();

    let result = db.create_collection("test", 768, DistanceMetric::Cosine);
    assert!(result.is_err());
}

#[test]
fn test_get_collection() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();

    // Non-existent collection returns None
    assert!(db.get_collection("nonexistent").is_none());

    // Create and retrieve collection
    db.create_collection("test", 768, DistanceMetric::Cosine)
        .unwrap();

    let collection = db.get_collection("test");
    assert!(collection.is_some());

    let config = collection.unwrap().config();
    assert_eq!(config.dimension, 768);
    assert_eq!(config.metric, DistanceMetric::Cosine);
}

#[test]
fn test_delete_collection() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();

    db.create_collection("to_delete", 768, DistanceMetric::Cosine)
        .unwrap();
    assert_eq!(db.list_collections().len(), 1);

    // Delete the collection
    db.delete_collection("to_delete").unwrap();
    assert!(db.list_collections().is_empty());
    assert!(db.get_collection("to_delete").is_none());
}

#[test]
fn test_delete_nonexistent_collection() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();

    let result = db.delete_collection("nonexistent");
    assert!(result.is_err());
}

#[test]
fn test_multiple_collections() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();

    db.create_collection("coll1", 128, DistanceMetric::Cosine)
        .unwrap();
    db.create_collection("coll2", 256, DistanceMetric::Euclidean)
        .unwrap();
    db.create_collection("coll3", 768, DistanceMetric::DotProduct)
        .unwrap();

    let collections = db.list_collections();
    assert_eq!(collections.len(), 3);
    assert!(collections.contains(&"coll1".to_string()));
    assert!(collections.contains(&"coll2".to_string()));
    assert!(collections.contains(&"coll3".to_string()));
}

#[test]
fn test_database_execute_query_join_on_end_to_end() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();

    db.create_collection("orders", 2, DistanceMetric::Cosine)
        .unwrap();
    db.create_collection("customers", 2, DistanceMetric::Cosine)
        .unwrap();

    let orders = db.get_collection("orders").unwrap();
    let customers = db.get_collection("customers").unwrap();

    orders
        .upsert(vec![
            Point::new(
                1,
                vec![1.0, 0.0],
                Some(serde_json::json!({"id": 1, "customer_id": 10, "total": 100})),
            ),
            Point::new(
                2,
                vec![0.0, 1.0],
                Some(serde_json::json!({"id": 2, "customer_id": 999, "total": 50})),
            ),
        ])
        .unwrap();
    customers
        .upsert(vec![Point::new(
            10,
            vec![1.0, 0.0],
            Some(serde_json::json!({"id": 10, "name": "Alice", "tier": "gold"})),
        )])
        .unwrap();

    let query =
        Parser::parse("SELECT * FROM orders JOIN customers ON orders.customer_id = customers.id")
            .unwrap();
    let results = db
        .execute_query(&query, &std::collections::HashMap::new())
        .unwrap();

    assert_eq!(results.len(), 1);
    let payload = results[0].point.payload.as_ref().unwrap();
    assert_eq!(payload.get("name").unwrap().as_str(), Some("Alice"));
}

#[test]
fn test_database_execute_query_join_using_with_graph_match_filter() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();

    db.create_collection("orders", 2, DistanceMetric::Cosine)
        .unwrap();
    db.create_collection("profiles", 2, DistanceMetric::Cosine)
        .unwrap();

    let orders = db.get_collection("orders").unwrap();
    let profiles = db.get_collection("profiles").unwrap();

    orders
        .upsert(vec![
            Point::new(
                1,
                vec![1.0, 0.0],
                Some(serde_json::json!({"id": 1, "_labels": ["Doc"], "kind": "source"})),
            ),
            Point::new(
                2,
                vec![0.0, 1.0],
                Some(serde_json::json!({"id": 2, "_labels": ["Doc"], "kind": "target"})),
            ),
        ])
        .unwrap();
    orders
        .add_edge(GraphEdge::new(100, 1, 2, "REL").unwrap())
        .unwrap();

    profiles
        .upsert(vec![
            Point::new(
                1,
                vec![1.0, 0.0],
                Some(serde_json::json!({"id": 1, "nickname": "alpha"})),
            ),
            Point::new(
                2,
                vec![0.0, 1.0],
                Some(serde_json::json!({"id": 2, "nickname": "beta"})),
            ),
        ])
        .unwrap();

    let query = Parser::parse(
        "SELECT * FROM orders AS o JOIN profiles USING (id) WHERE MATCH (o:Doc)-[:REL]->(x:Doc)",
    )
    .unwrap();
    let results = db
        .execute_query(&query, &std::collections::HashMap::new())
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].point.id, 1);
    let payload = results[0].point.payload.as_ref().unwrap();
    assert_eq!(payload.get("nickname").unwrap().as_str(), Some("alpha"));
}

#[test]
fn test_database_execute_query_supports_left_join_runtime() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    db.create_collection("orders", 2, DistanceMetric::Cosine)
        .unwrap();
    db.create_collection("customers", 2, DistanceMetric::Cosine)
        .unwrap();

    let orders = db.get_collection("orders").unwrap();
    orders
        .upsert(vec![Point::new(
            1,
            vec![1.0, 0.0],
            Some(serde_json::json!({"customer_id": 999})),
        )])
        .unwrap();

    let query = Parser::parse(
        "SELECT * FROM orders LEFT JOIN customers ON customers.id = orders.customer_id",
    )
    .unwrap();
    let results = db
        .execute_query(&query, &std::collections::HashMap::new())
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].point.id, 1);
    let payload = results[0].point.payload.as_ref().unwrap();
    assert_eq!(payload.get("customer_id"), Some(&serde_json::json!(999)));
    assert_eq!(payload.get("id"), Some(&serde_json::Value::Null));
}

#[test]
fn test_database_execute_query_rejects_join_using_multi_column() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    db.create_collection("orders", 2, DistanceMetric::Cosine)
        .unwrap();
    db.create_collection("customers", 2, DistanceMetric::Cosine)
        .unwrap();

    let query =
        Parser::parse("SELECT * FROM orders JOIN customers USING (id, customer_id)").unwrap();
    let err = db
        .execute_query(&query, &std::collections::HashMap::new())
        .unwrap_err();
    assert!(err.to_string().contains("USING(single_column)"));
}

#[test]
fn test_collection_execute_query_match_order_by_property() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    db.create_collection("docs", 2, DistanceMetric::Cosine)
        .unwrap();
    let docs = db.get_collection("docs").unwrap();

    docs.upsert(vec![
        Point::new(
            1,
            vec![1.0, 0.0],
            Some(serde_json::json!({"_labels": ["Doc"], "name": "Charlie"})),
        ),
        Point::new(
            2,
            vec![1.0, 0.0],
            Some(serde_json::json!({"_labels": ["Doc"], "name": "Alice"})),
        ),
        Point::new(
            3,
            vec![1.0, 0.0],
            Some(serde_json::json!({"_labels": ["Doc"], "name": "Bob"})),
        ),
    ])
    .unwrap();

    let query = Parser::parse("MATCH (d:Doc) RETURN d.name ORDER BY d.name ASC LIMIT 3").unwrap();
    let results = docs
        .execute_query(&query, &std::collections::HashMap::new())
        .unwrap();

    let names: Vec<String> = results
        .iter()
        .map(|r| {
            r.point
                .payload
                .as_ref()
                .and_then(|p| p.get("name"))
                .and_then(serde_json::Value::as_str)
                .unwrap()
                .to_string()
        })
        .collect();
    assert_eq!(names, vec!["Alice", "Bob", "Charlie"]);
}

#[test]
fn test_database_execute_query_rejects_top_level_match_queries() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    db.create_collection("docs", 2, DistanceMetric::Cosine)
        .unwrap();

    let query = Parser::parse("MATCH (d:Doc) RETURN d LIMIT 10").unwrap();
    let err = db
        .execute_query(&query, &std::collections::HashMap::new())
        .unwrap_err();
    assert!(err
        .to_string()
        .contains("Database::execute_query does not support top-level MATCH queries"));
}

#[test]
fn test_database_execute_query_insert_metadata_only() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    db.create_collection_typed("products", &CollectionType::MetadataOnly)
        .unwrap();

    let query = Parser::parse(
        "INSERT INTO products (id, name, price, active) VALUES (1, 'Notebook', 12.5, true)",
    )
    .unwrap();
    let results = db
        .execute_query(&query, &std::collections::HashMap::new())
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].point.id, 1);
    let payload = results[0].point.payload.as_ref().unwrap();
    assert_eq!(payload["name"], serde_json::json!("Notebook"));
    assert_eq!(payload["price"], serde_json::json!(12.5));
    assert_eq!(payload["active"], serde_json::json!(true));
}

#[test]
fn test_database_execute_query_update_metadata_only_where_id() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    db.create_collection_typed("products", &CollectionType::MetadataOnly)
        .unwrap();
    let products = db.get_collection("products").unwrap();
    products
        .upsert_metadata(vec![Point::metadata_only(
            1,
            serde_json::json!({"name": "Notebook", "price": 10.0}),
        )])
        .unwrap();

    let query = Parser::parse("UPDATE products SET price = 19.99 WHERE id = 1").unwrap();
    let results = db
        .execute_query(&query, &std::collections::HashMap::new())
        .unwrap();
    assert_eq!(results.len(), 1);

    let updated = products.get(&[1]).into_iter().flatten().next().unwrap();
    let payload = updated.payload.unwrap();
    assert_eq!(payload["price"], serde_json::json!(19.99));
}

#[test]
fn test_database_execute_query_insert_with_params() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    db.create_collection_typed("profiles", &CollectionType::MetadataOnly)
        .unwrap();

    let query =
        Parser::parse("INSERT INTO profiles (id, name, age) VALUES ($id, $name, $age)").unwrap();
    let mut params = std::collections::HashMap::new();
    params.insert("id".to_string(), serde_json::json!(7));
    params.insert("name".to_string(), serde_json::json!("Alice"));
    params.insert("age".to_string(), serde_json::json!(30));

    db.execute_query(&query, &params).unwrap();

    let profiles = db.get_collection("profiles").unwrap();
    let point = profiles.get(&[7]).into_iter().flatten().next().unwrap();
    let payload = point.payload.unwrap();
    assert_eq!(payload["name"], serde_json::json!("Alice"));
    assert_eq!(payload["age"], serde_json::json!(30));
}
