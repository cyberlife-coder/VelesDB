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

    // Use get_collection() here to get the shared instance that supports both
    // vector operations and graph operations (add_edge) on the same Collection.
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
    let docs = db.get_vector_collection("docs").unwrap();

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
    // get_collection() returns the shared registry instance for INSERT to be visible
    let _profiles = db.get_collection("profiles").unwrap();

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

#[test]
fn test_create_collection_rejects_existing_on_disk_dir_not_loaded() {
    let dir = tempdir().unwrap();
    let coll_dir = dir.path().join("orphaned");
    std::fs::create_dir_all(&coll_dir).unwrap();
    // Simulate a corrupted collection that load_collections() skips.
    std::fs::write(coll_dir.join("config.json"), "{invalid json").unwrap();

    let db = Database::open(dir.path()).unwrap();
    let result = db.create_collection("orphaned", 8, DistanceMetric::Cosine);
    assert!(matches!(result, Err(Error::CollectionExists(_))));
}

// ---------------------------------------------------------------------------
// execute_train tests
// ---------------------------------------------------------------------------

/// Helper: insert vectors into a collection for training.
fn seed_training_vectors(db: &Database, name: &str, dim: usize, count: usize) {
    let coll = db.get_collection(name).unwrap();
    let points: Vec<Point> = (0..count)
        .map(|i| {
            #[allow(clippy::cast_precision_loss)]
            let v: Vec<f32> = (0..dim)
                .map(|d| ((i * 31 + d * 17 + 11) % 1000) as f32 / 1000.0)
                .collect();
            Point::new(i as u64, v, Some(serde_json::json!({})))
        })
        .collect();
    coll.upsert(points).unwrap();
}

#[test]
fn test_execute_train_pq_success() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    // dimension=16, m=4 => 16 % 4 == 0
    db.create_collection("docs", 16, DistanceMetric::Euclidean)
        .unwrap();
    seed_training_vectors(&db, "docs", 16, 300);

    let query = Parser::parse("TRAIN QUANTIZER ON docs WITH (m=4, k=16)").unwrap();
    let results = db
        .execute_query(&query, &std::collections::HashMap::new())
        .unwrap();

    assert_eq!(results.len(), 1);
    let payload = results[0].point.payload.as_ref().unwrap();
    assert_eq!(payload["status"], serde_json::json!("trained"));
    assert_eq!(payload["type"], serde_json::json!("pq"));

    // Verify storage mode updated
    let coll = db.get_collection("docs").unwrap();
    assert_eq!(coll.config().storage_mode, StorageMode::ProductQuantization);
}

#[test]
fn test_execute_train_collection_not_found() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();

    let query = Parser::parse("TRAIN QUANTIZER ON nonexistent WITH (m=4, k=16)").unwrap();
    let err = db
        .execute_query(&query, &std::collections::HashMap::new())
        .unwrap_err();
    assert!(matches!(err, Error::CollectionNotFound(_)));
}

#[test]
fn test_execute_train_invalid_m_zero() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    db.create_collection("docs", 16, DistanceMetric::Euclidean)
        .unwrap();
    seed_training_vectors(&db, "docs", 16, 100);

    let query = Parser::parse("TRAIN QUANTIZER ON docs WITH (m=0, k=16)").unwrap();
    let err = db
        .execute_query(&query, &std::collections::HashMap::new())
        .unwrap_err();
    assert!(matches!(err, Error::InvalidQuantizerConfig(_)));
}

#[test]
fn test_execute_train_opq_success() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    db.create_collection("vecs", 16, DistanceMetric::Euclidean)
        .unwrap();
    seed_training_vectors(&db, "vecs", 16, 300);

    let query = Parser::parse("TRAIN QUANTIZER ON vecs WITH (m=4, k=16, type=opq)").unwrap();
    let results = db
        .execute_query(&query, &std::collections::HashMap::new())
        .unwrap();

    assert_eq!(results.len(), 1);
    let payload = results[0].point.payload.as_ref().unwrap();
    assert_eq!(payload["type"], serde_json::json!("opq"));
    assert_eq!(payload["status"], serde_json::json!("trained"));

    let coll = db.get_collection("vecs").unwrap();
    assert_eq!(coll.config().storage_mode, StorageMode::ProductQuantization);
}

#[test]
fn test_execute_train_rabitq_success() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    db.create_collection("rbq", 32, DistanceMetric::Euclidean)
        .unwrap();
    seed_training_vectors(&db, "rbq", 32, 100);

    let query = Parser::parse("TRAIN QUANTIZER ON rbq WITH (m=4, type=rabitq)").unwrap();
    let results = db
        .execute_query(&query, &std::collections::HashMap::new())
        .unwrap();

    assert_eq!(results.len(), 1);
    let payload = results[0].point.payload.as_ref().unwrap();
    assert_eq!(payload["type"], serde_json::json!("rabitq"));

    let coll = db.get_collection("rbq").unwrap();
    assert_eq!(coll.config().storage_mode, StorageMode::RaBitQ);
}

#[test]
fn test_execute_train_updates_storage_mode() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    db.create_collection("docs", 16, DistanceMetric::Euclidean)
        .unwrap();
    seed_training_vectors(&db, "docs", 16, 300);

    // Verify initial state
    let coll = db.get_collection("docs").unwrap();
    assert_eq!(coll.config().storage_mode, StorageMode::Full);

    let query = Parser::parse("TRAIN QUANTIZER ON docs WITH (m=4, k=16)").unwrap();
    db.execute_query(&query, &std::collections::HashMap::new())
        .unwrap();

    // After training
    let coll = db.get_collection("docs").unwrap();
    assert_eq!(coll.config().storage_mode, StorageMode::ProductQuantization);
}

#[test]
fn test_execute_train_dim_not_divisible_by_m() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    // dim=15, m=4 => 15 % 4 != 0
    db.create_collection("bad", 15, DistanceMetric::Euclidean)
        .unwrap();
    seed_training_vectors(&db, "bad", 15, 100);

    let query = Parser::parse("TRAIN QUANTIZER ON bad WITH (m=4, k=16)").unwrap();
    let err = db
        .execute_query(&query, &std::collections::HashMap::new())
        .unwrap_err();
    // This should be caught either by our validation or by ProductQuantizer::train
    assert!(
        matches!(
            err,
            Error::InvalidQuantizerConfig(_) | Error::TrainingFailed(_)
        ),
        "Expected InvalidQuantizerConfig or TrainingFailed, got: {err:?}"
    );
}

#[test]
fn test_execute_train_rejects_retrain_without_force() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    db.create_collection("docs", 16, DistanceMetric::Euclidean)
        .unwrap();
    seed_training_vectors(&db, "docs", 16, 300);

    // First training succeeds
    let query = Parser::parse("TRAIN QUANTIZER ON docs WITH (m=4, k=16)").unwrap();
    db.execute_query(&query, &std::collections::HashMap::new())
        .unwrap();

    // Second training without force=true should fail
    let query2 = Parser::parse("TRAIN QUANTIZER ON docs WITH (m=4, k=16)").unwrap();
    let err = db
        .execute_query(&query2, &std::collections::HashMap::new())
        .unwrap_err();
    assert!(matches!(err, Error::InvalidQuantizerConfig(_)));
    assert!(err.to_string().contains("already trained"));
}

#[test]
fn test_execute_train_force_retrain_succeeds() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    db.create_collection("docs", 16, DistanceMetric::Euclidean)
        .unwrap();
    seed_training_vectors(&db, "docs", 16, 300);

    // First training
    let query = Parser::parse("TRAIN QUANTIZER ON docs WITH (m=4, k=16)").unwrap();
    db.execute_query(&query, &std::collections::HashMap::new())
        .unwrap();

    // Retrain with force=true
    let query2 = Parser::parse("TRAIN QUANTIZER ON docs WITH (m=4, k=16, force=true)").unwrap();
    let results = db
        .execute_query(&query2, &std::collections::HashMap::new())
        .unwrap();
    assert_eq!(results.len(), 1);
    let payload = results[0].point.payload.as_ref().unwrap();
    assert_eq!(payload["status"], serde_json::json!("trained"));
}

#[test]
fn test_execute_train_with_sample_limit() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    db.create_collection("docs", 16, DistanceMetric::Euclidean)
        .unwrap();
    seed_training_vectors(&db, "docs", 16, 500);

    // Train with sample=100 (fewer than the 500 vectors available)
    let query = Parser::parse("TRAIN QUANTIZER ON docs WITH (m=4, k=16, sample=100)").unwrap();
    let results = db
        .execute_query(&query, &std::collections::HashMap::new())
        .unwrap();
    assert_eq!(results.len(), 1);
    let payload = results[0].point.payload.as_ref().unwrap();
    assert_eq!(payload["training_vectors"], serde_json::json!(100));
}

#[test]
fn test_execute_train_sample_larger_than_dataset() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    db.create_collection("docs", 16, DistanceMetric::Euclidean)
        .unwrap();
    seed_training_vectors(&db, "docs", 16, 200);

    // sample=9999 but only 200 vectors — should use all 200
    let query = Parser::parse("TRAIN QUANTIZER ON docs WITH (m=4, k=16, sample=9999)").unwrap();
    let results = db
        .execute_query(&query, &std::collections::HashMap::new())
        .unwrap();
    let payload = results[0].point.payload.as_ref().unwrap();
    assert_eq!(payload["training_vectors"], serde_json::json!(200));
}

#[test]
fn test_execute_train_missing_m_is_required() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    db.create_collection("docs", 16, DistanceMetric::Euclidean)
        .unwrap();
    seed_training_vectors(&db, "docs", 16, 100);

    // No m parameter at all
    let query = Parser::parse("TRAIN QUANTIZER ON docs WITH (k=16)").unwrap();
    let err = db
        .execute_query(&query, &std::collections::HashMap::new())
        .unwrap_err();
    assert!(matches!(err, Error::InvalidQuantizerConfig(_)));
    assert!(err.to_string().contains("must be > 0"));
}

#[test]
#[allow(clippy::cast_possible_truncation)]
fn test_database_open_loads_sparse_index() {
    use crate::index::sparse::persistence::{compact, wal_append_upsert};
    use crate::index::sparse::types::SparseVector;
    use crate::index::sparse::SparseInvertedIndex;

    let dir = tempdir().unwrap();

    // Step 1: Create a collection via Database API
    {
        let db = Database::open(dir.path()).unwrap();
        db.create_collection("sparse_test", 4, DistanceMetric::Cosine)
            .unwrap();
    }

    // Step 2: Manually write sparse index files into collection directory
    let coll_dir = dir.path().join("sparse_test");
    {
        let idx = SparseInvertedIndex::new();
        for i in 0..20u64 {
            let v = SparseVector::new(vec![(1, 1.0), (2 + i as u32 % 5, 0.5)]);
            idx.insert(i, &v);
        }
        compact(&coll_dir, &idx).unwrap();
    }

    // Step 3: Reopen database and verify sparse index is loaded
    {
        let db = Database::open(dir.path()).unwrap();
        let coll = db.get_collection("sparse_test").unwrap();
        let guard = coll.sparse_indexes().read();
        assert!(
            guard.contains_key(""),
            "Default sparse index should be loaded from disk on Database::open()"
        );
        let sparse = guard.get("").unwrap();
        assert_eq!(sparse.doc_count(), 20);
    }

    // Step 4: Test with WAL-only scenario (no compacted files)
    let dir2 = tempdir().unwrap();
    {
        let db = Database::open(dir2.path()).unwrap();
        db.create_collection("wal_only", 4, DistanceMetric::Cosine)
            .unwrap();
    }
    let coll_dir2 = dir2.path().join("wal_only");
    {
        let wal_path = coll_dir2.join("sparse.wal");
        for i in 0..5u64 {
            let v = SparseVector::new(vec![(1, 1.0)]);
            wal_append_upsert(&wal_path, i, &v).unwrap();
        }
    }
    {
        let db = Database::open(dir2.path()).unwrap();
        let coll = db.get_collection("wal_only").unwrap();
        let guard = coll.sparse_indexes().read();
        assert!(
            guard.contains_key(""),
            "WAL-only sparse index should be loaded on Database::open()"
        );
        assert_eq!(guard.get("").unwrap().doc_count(), 5);
    }
}
