use super::*;
use crate::collection::graph::GraphEdge;
use crate::point::Point;
use crate::velesql::Parser;
use crate::{CollectionType, DistanceMetric, StorageMode};
use tempfile::tempdir;

#[test]
fn test_database_open() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    assert!(db.list_collections().is_empty());
}

#[test]
fn test_open_with_config_rejects_invalid_config() {
    // #907 follow-up: a `VelesConfig` built programmatically (bypassing the
    // loader, which is the only place that used to call validate()) must be
    // validated at the open boundary, so an out-of-range field is rejected.
    let dir = tempdir().unwrap();
    let mut config = crate::config::VelesConfig::default();
    config.limits.max_collections = 0; // out of range [1, ..]

    let result = Database::open_with_config(dir.path(), config);
    assert!(
        result.is_err(),
        "open_with_config must reject an invalid (unloader-validated) config"
    );
    let msg = result.err().unwrap().to_string();
    assert!(
        msg.contains("limits.max_collections"),
        "error should name the offending key, got: {msg}"
    );
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
    assert!(db.get_vector_collection("nonexistent").is_none());

    // Create and retrieve collection
    db.create_collection("test", 768, DistanceMetric::Cosine)
        .unwrap();

    let collection = db.get_vector_collection("test");
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
    assert!(db.get_vector_collection("to_delete").is_none());
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

    let orders = db.get_vector_collection("orders").unwrap();
    let customers = db.get_vector_collection("customers").unwrap();

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

    // Use get_vector_collection() here to get the shared instance that supports both
    // vector operations and graph operations (add_edge) on the same Collection.
    let orders = db.get_vector_collection("orders").unwrap();
    let profiles = db.get_vector_collection("profiles").unwrap();

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

    let orders = db.get_vector_collection("orders").unwrap();
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

    // MATCH without FROM or _collection param → clear guidance error
    let query = Parser::parse("MATCH (d:Doc) RETURN d LIMIT 10").unwrap();
    let err = db
        .execute_query(&query, &std::collections::HashMap::new())
        .unwrap_err();
    assert!(
        err.to_string().contains("target collection"),
        "Should guide user to specify collection, got: {}",
        err
    );
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
    let products = db.get_metadata_collection("products").unwrap();
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
    // get_vector_collection() returns the shared registry instance for INSERT to be visible
    let _profiles = db.get_metadata_collection("profiles").unwrap();

    let query =
        Parser::parse("INSERT INTO profiles (id, name, age) VALUES ($id, $name, $age)").unwrap();
    let mut params = std::collections::HashMap::new();
    params.insert("id".to_string(), serde_json::json!(7));
    params.insert("name".to_string(), serde_json::json!("Alice"));
    params.insert("age".to_string(), serde_json::json!(30));

    db.execute_query(&query, &params).unwrap();

    let profiles = db.get_metadata_collection("profiles").unwrap();
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
    let coll = db.get_vector_collection(name).unwrap();
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
    let coll = db.get_vector_collection("docs").unwrap();
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

    let coll = db.get_vector_collection("vecs").unwrap();
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

    let coll = db.get_vector_collection("rbq").unwrap();
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
    let coll = db.get_vector_collection("docs").unwrap();
    assert_eq!(coll.config().storage_mode, StorageMode::Full);

    let query = Parser::parse("TRAIN QUANTIZER ON docs WITH (m=4, k=16)").unwrap();
    db.execute_query(&query, &std::collections::HashMap::new())
        .unwrap();

    // After training
    let coll = db.get_vector_collection("docs").unwrap();
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

// ---------------------------------------------------------------------------
// Quantizer wiring across restarts (RaBitQ + PQ persistence round-trips)
// ---------------------------------------------------------------------------

/// Builds a sinusoidal vector — spread-out distances, no symmetric ties,
/// so brute-force/ANN top-k comparisons are stable.
fn sin_vector(dim: usize, i: usize) -> Vec<f32> {
    #[allow(clippy::cast_precision_loss)]
    (0..dim)
        .map(|d| ((i * dim + d) as f32 * 0.01).sin())
        .collect()
}

/// Inserts `count` sinusoidal vectors (ids `0..count`) into a collection.
fn seed_sin_vectors(db: &Database, name: &str, dim: usize, count: usize) {
    let coll = db.get_vector_collection(name).unwrap();
    let points: Vec<Point> = (0..count)
        .map(|i| Point::new(i as u64, sin_vector(dim, i), Some(serde_json::json!({}))))
        .collect();
    coll.upsert(points).unwrap();
}

/// Brute-force Euclidean top-k ids over the seeded sinusoidal set.
fn sin_brute_force_top_k(
    query: &[f32],
    dim: usize,
    count: usize,
    k: usize,
) -> std::collections::HashSet<u64> {
    let mut dists: Vec<(u64, f32)> = (0..count)
        .map(|i| {
            let v = sin_vector(dim, i);
            let d: f32 = query.iter().zip(&v).map(|(a, b)| (a - b) * (a - b)).sum();
            (i as u64, d)
        })
        .collect();
    dists.sort_by(|a, b| a.1.total_cmp(&b.1));
    dists.into_iter().take(k).map(|(id, _)| id).collect()
}

/// `TRAIN QUANTIZER 'rabitq'` on a collection created with `storage='rabitq'`
/// must install the quantizer into the live backend (no restart needed).
#[test]
fn test_execute_train_rabitq_installs_into_live_backend() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    db.create_collection_with_options(
        "rbq_live",
        64,
        DistanceMetric::Euclidean,
        StorageMode::RaBitQ,
    )
    .unwrap();
    seed_sin_vectors(&db, "rbq_live", 64, 300);

    let coll = db.resolve_writable_collection("rbq_live").unwrap();
    assert!(
        !coll.is_rabitq_quantizer_trained(),
        "300 inserts stay below the lazy-train threshold"
    );

    let query = Parser::parse("TRAIN QUANTIZER ON rbq_live WITH (m=4, type=rabitq)").unwrap();
    db.execute_query(&query, &std::collections::HashMap::new())
        .unwrap();

    assert!(
        coll.is_rabitq_quantizer_trained(),
        "TRAIN must install the quantizer into the live RaBitQ backend"
    );

    let results = coll.search(&sin_vector(64, 42), 5).unwrap();
    assert_eq!(
        results.first().map(|r| r.point.id),
        Some(42),
        "self-query must return itself as top-1 through the RaBitQ path"
    );
}

/// End-to-end restart wiring: create → insert → TRAIN 'rabitq' → reopen the
/// Database → the trained quantizer must be restored from `rabitq.idx` and
/// search must keep recall parity with brute force.
#[test]
fn test_train_rabitq_wiring_survives_reopen() {
    let dir = tempdir().unwrap();
    {
        let db = Database::open(dir.path()).unwrap();
        // Created with the default Full mode: TRAIN flips the config to
        // RaBitQ and the backend takes effect at the reopen below.
        db.create_collection("rbq_reopen", 64, DistanceMetric::Euclidean)
            .unwrap();
        seed_sin_vectors(&db, "rbq_reopen", 64, 300);
        let query = Parser::parse("TRAIN QUANTIZER ON rbq_reopen WITH (m=4, type=rabitq)").unwrap();
        db.execute_query(&query, &std::collections::HashMap::new())
            .unwrap();
        assert_eq!(db.flush_all(), 0, "flush before reopen must succeed");
    }

    let db = Database::open(dir.path()).unwrap();
    let coll = db.resolve_writable_collection("rbq_reopen").unwrap();
    assert_eq!(coll.config().storage_mode, StorageMode::RaBitQ);
    assert!(
        coll.is_rabitq_quantizer_trained(),
        "rabitq.idx must be reloaded and installed on open"
    );

    // Recall parity with brute force (set overlap, not exact scores).
    let query_vec = sin_vector(64, 42);
    let results = coll.search(&query_vec, 10).unwrap();
    assert_eq!(results.len(), 10);
    let result_ids: std::collections::HashSet<u64> = results.iter().map(|r| r.point.id).collect();
    let brute_ids = sin_brute_force_top_k(&query_vec, 64, 300, 10);
    let overlap = brute_ids.intersection(&result_ids).count();
    // Same bar as rabitq_precision_tests: exact-f32 rerank over oversampled
    // candidates on a small set must be near-perfect; anything lower hides
    // partial store misalignment.
    assert!(
        overlap >= 9,
        "recall@10 vs brute force should be >= 0.9 after reopen, got {overlap}/10"
    );
}

/// The persisted TRAIN artifact must survive a reopen even when the
/// collection exceeds the lazy-training threshold (1000 vectors): gap
/// recovery re-inserts every vector on open, and without the pre-recovery
/// install those inserts would lazily train a throwaway quantizer that
/// preempts `rabitq.idx` (review 2026-06-11 finding 1).
#[test]
fn test_train_rabitq_survives_reopen_beyond_lazy_threshold() {
    let dir = tempdir().unwrap();
    {
        let db = Database::open(dir.path()).unwrap();
        db.create_collection("rbq_large", 32, DistanceMetric::Euclidean)
            .unwrap();
        seed_sin_vectors(&db, "rbq_large", 32, 1200);
        let query = Parser::parse("TRAIN QUANTIZER ON rbq_large WITH (m=4, type=rabitq)").unwrap();
        db.execute_query(&query, &std::collections::HashMap::new())
            .unwrap();
        assert_eq!(db.flush_all(), 0, "flush before reopen must succeed");
    }

    let db = Database::open(dir.path()).unwrap();
    let coll = db.resolve_writable_collection("rbq_large").unwrap();
    assert!(
        coll.is_rabitq_quantizer_trained(),
        "the persisted quantizer must be installed before gap recovery"
    );

    let query_vec = sin_vector(32, 7);
    let results = coll.search(&query_vec, 10).unwrap();
    assert_eq!(results.len(), 10);
    let result_ids: std::collections::HashSet<u64> = results.iter().map(|r| r.point.id).collect();
    let brute_ids = sin_brute_force_top_k(&query_vec, 32, 1200, 10);
    let overlap = brute_ids.intersection(&result_ids).count();
    assert!(
        overlap >= 9,
        "recall@10 vs brute force should be >= 0.9 after large reopen, got {overlap}/10"
    );
}

/// A `codebook.pq` whose dimension does not match the collection must be
/// rejected at open with a single warning (clean f32 fallback) instead of
/// silently producing an empty PQ cache via per-vector encode failures.
#[test]
fn test_foreign_pq_codebook_dimension_rejected_on_open() {
    let dir = tempdir().unwrap();
    {
        let db = Database::open(dir.path()).unwrap();
        db.create_collection_with_options(
            "pq_mismatch",
            16,
            DistanceMetric::Euclidean,
            StorageMode::ProductQuantization,
        )
        .unwrap();
        seed_sin_vectors(&db, "pq_mismatch", 16, 50);
        assert_eq!(db.flush_all(), 0, "flush before reopen must succeed");

        // Plant a foreign 32-dim codebook into the collection directory
        // AFTER the flush so nothing overwrites it.
        let coll = db.resolve_writable_collection("pq_mismatch").unwrap();
        let foreign: Vec<Vec<f32>> = (0..64).map(|i| sin_vector(32, i)).collect();
        let pq = crate::quantization::ProductQuantizer::train(&foreign, 4, 16).unwrap();
        pq.save_codebook(coll.data_path()).unwrap();
    }

    let db = Database::open(dir.path()).unwrap();
    let coll = db.resolve_writable_collection("pq_mismatch").unwrap();
    assert!(
        coll.pq_quantizer_read().is_none(),
        "mismatched codebook must not be installed"
    );
    assert_eq!(
        coll.pq_cache_len(),
        0,
        "no PQ cache must be rebuilt from a foreign codebook"
    );
    // Search keeps working on the exact f32 path.
    let results = coll.search(&sin_vector(16, 7), 5).unwrap();
    assert!(!results.is_empty(), "f32 fallback search must keep working");
}

/// A lazily-trained `RaBitQ` quantizer (1000-insert threshold, no explicit
/// `TRAIN QUANTIZER`) must be persisted to `rabitq.idx` by the full flush and
/// reinstalled on reopen, instead of silently degrading to f32 search
/// (parity with the PQ codebook flush).
#[test]
fn test_lazy_trained_rabitq_survives_reopen_via_flush() {
    let dir = tempdir().unwrap();
    {
        let db = Database::open(dir.path()).unwrap();
        db.create_collection_with_options(
            "rbq_lazy",
            32,
            DistanceMetric::Euclidean,
            StorageMode::RaBitQ,
        )
        .unwrap();
        seed_sin_vectors(&db, "rbq_lazy", 32, 1200);
        let coll = db.resolve_writable_collection("rbq_lazy").unwrap();
        assert!(
            coll.is_rabitq_quantizer_trained(),
            "1200 inserts must cross the lazy-train threshold"
        );
        assert_eq!(db.flush_all(), 0, "flush before reopen must succeed");
        assert!(
            coll.data_path().join("rabitq.idx").exists(),
            "full flush must persist the lazily-trained quantizer"
        );
    }

    let db = Database::open(dir.path()).unwrap();
    let coll = db.resolve_writable_collection("rbq_lazy").unwrap();
    assert!(
        coll.is_rabitq_quantizer_trained(),
        "lazily-trained quantizer must be reinstalled from rabitq.idx on open"
    );

    let query_vec = sin_vector(32, 7);
    let results = coll.search(&query_vec, 10).unwrap();
    assert_eq!(results.len(), 10);
    let result_ids: std::collections::HashSet<u64> = results.iter().map(|r| r.point.id).collect();
    let brute_ids = sin_brute_force_top_k(&query_vec, 32, 1200, 10);
    let overlap = brute_ids.intersection(&result_ids).count();
    assert!(
        overlap >= 9,
        "recall@10 vs brute force should be >= 0.9 after lazy reopen, got {overlap}/10"
    );
}

/// PQ persistence round-trip: the codebook saved by `TRAIN QUANTIZER` must be
/// reloaded on open and the PQ cache rebuilt, so the ADC rescore path stays
/// live after a restart.
#[test]
fn test_train_pq_codebook_and_cache_survive_reopen() {
    let dir = tempdir().unwrap();
    {
        let db = Database::open(dir.path()).unwrap();
        // storage='pq' from creation: inserts lazily train an in-memory
        // quantizer after 128 points, but only TRAIN persists the codebook —
        // force=true replaces the lazily trained one.
        db.create_collection_with_options(
            "pqc",
            16,
            DistanceMetric::Euclidean,
            StorageMode::ProductQuantization,
        )
        .unwrap();
        seed_sin_vectors(&db, "pqc", 16, 300);
        let query = Parser::parse("TRAIN QUANTIZER ON pqc WITH (m=4, k=16, force=true)").unwrap();
        db.execute_query(&query, &std::collections::HashMap::new())
            .unwrap();
        assert_eq!(db.flush_all(), 0, "flush before reopen must succeed");
    }

    let db = Database::open(dir.path()).unwrap();
    let coll = db.resolve_writable_collection("pqc").unwrap();
    assert_eq!(coll.config().storage_mode, StorageMode::ProductQuantization);
    assert!(
        coll.pq_quantizer_read().is_some(),
        "codebook.pq must be reloaded on open"
    );
    assert_eq!(
        coll.pq_cache_len(),
        300,
        "PQ cache must be rebuilt for every stored vector"
    );

    // ADC-rescored search still finds the self-query (PQ is lossy: top-10,
    // not top-1).
    let results = coll.search(&sin_vector(16, 42), 10).unwrap();
    assert!(
        results.iter().any(|r| r.point.id == 42),
        "self-query must appear in PQ-rescored top-10 after reopen"
    );
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
        let coll = db.get_vector_collection("sparse_test").unwrap();
        let guard = coll.inner.sparse_indexes().read();
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
        let coll = db.get_vector_collection("wal_only").unwrap();
        let guard = coll.inner.sparse_indexes().read();
        assert!(
            guard.contains_key(""),
            "WAL-only sparse index should be loaded on Database::open()"
        );
        assert_eq!(guard.get("").unwrap().doc_count(), 5);
    }
}

#[test]
fn test_update_guardrails_propagates_to_collections() {
    use crate::guardrails::QueryLimits;

    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();

    db.create_collection("coll_a", 128, DistanceMetric::Cosine)
        .unwrap();
    db.create_collection("coll_b", 64, DistanceMetric::Euclidean)
        .unwrap();

    // Default timeout is 30_000 ms.
    let coll_a = db.get_vector_collection("coll_a").unwrap();
    assert_eq!(coll_a.guard_rails().limits().timeout_ms, 30_000);

    // Update guardrails at the database level.
    let new_limits = QueryLimits::default()
        .with_timeout_ms(5_000)
        .with_max_depth(3);
    db.update_guardrails(&new_limits);

    // Both collections should reflect the updated limits.
    let coll_a = db.get_vector_collection("coll_a").unwrap();
    let coll_b = db.get_vector_collection("coll_b").unwrap();
    assert_eq!(coll_a.guard_rails().limits().timeout_ms, 5_000);
    assert_eq!(coll_a.guard_rails().limits().max_depth, 3);
    assert_eq!(coll_b.guard_rails().limits().timeout_ms, 5_000);
    assert_eq!(coll_b.guard_rails().limits().max_depth, 3);
}

#[test]
fn test_update_guardrails_affects_query_context() {
    use crate::guardrails::QueryLimits;

    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    db.create_collection("ctx_test", 128, DistanceMetric::Cosine)
        .unwrap();

    // Tighten the max_depth to 2.
    let new_limits = QueryLimits::default().with_max_depth(2);
    db.update_guardrails(&new_limits);

    // A query context created after the update should enforce the new limit.
    let coll = db.get_vector_collection("ctx_test").unwrap();
    let ctx = coll.guard_rails().create_context();
    assert!(ctx.check_depth(2).is_ok());
    assert!(ctx.check_depth(3).is_err());
}

// =========================================================================
// US-001: File locking tests
// =========================================================================

#[test]
fn test_database_file_locking_prevents_second_open() {
    let dir = tempdir().unwrap();
    let _db1 = Database::open(dir.path()).unwrap();

    // Second open on the same path must fail with DatabaseLocked
    let result = Database::open(dir.path());
    match result {
        Ok(_) => panic!("Expected DatabaseLocked error, got Ok"),
        Err(err) => assert!(
            matches!(err, crate::Error::DatabaseLocked(_)),
            "Expected DatabaseLocked, got: {err:?}"
        ),
    }
}

#[test]
fn test_database_lock_released_on_drop() {
    let dir = tempdir().unwrap();

    {
        let _db = Database::open(dir.path()).unwrap();
        // lock held inside this scope
    }
    // After drop, the lock should be released
    let _db2 = Database::open(dir.path()).unwrap();
}

// =========================================================================
// US-006: Collection diagnostics tests
// =========================================================================

#[test]
fn test_diagnostics_healthy_vector_collection() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    db.create_vector_collection("diag_test", 4, DistanceMetric::Cosine)
        .unwrap();

    let coll = db.get_vector_collection("diag_test").unwrap();
    coll.upsert(vec![Point::new(1, vec![0.1, 0.2, 0.3, 0.4], None)])
        .unwrap();

    let diag = coll.diagnostics();
    assert!(diag.has_vectors);
    assert!(diag.search_ready);
    assert!(diag.dimension_configured);
    assert_eq!(diag.point_count, 1);
    assert_eq!(diag.index_health, crate::collection::IndexHealth::Healthy);
}

#[test]
fn test_diagnostics_empty_vector_collection() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    db.create_vector_collection("empty_diag", 4, DistanceMetric::Cosine)
        .unwrap();

    let diag = db.collection_diagnostics("empty_diag").unwrap();
    assert!(!diag.has_vectors);
    assert!(!diag.search_ready);
    assert!(diag.dimension_configured);
    assert_eq!(diag.point_count, 0);
    assert_eq!(diag.index_health, crate::collection::IndexHealth::Empty);
}

#[test]
fn test_diagnostics_not_found() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    let result = db.collection_diagnostics("nonexistent");
    assert!(result.is_err());
}

// =========================================================================
// VelesConfig wiring — Wave 3 Commit 6
//
// The root config used to be defined in `config.rs` and loaded only by
// server/CLI code; `Database::open` ignored it entirely. Commit 6 wires
// a `config: Arc<VelesConfig>` field into `Database` and exposes the
// `open_with_config` / `open_with_observer_and_config` constructors.
// These tests anchor the happy path, the default fallback, and the
// Arc-based accessor contract so later commits (7, 8, 9) can rely on
// `db.config()` in every sub-system with confidence.
// =========================================================================

#[test]
fn test_database_open_default_config_matches_veles_config_default() {
    use crate::config::VelesConfig;

    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();

    // `Database::open` must install the exact same `VelesConfig::default()`
    // a user would get by calling the public builder — otherwise the
    // "same behaviour as pre-Wave-3" guarantee is broken.
    let default = VelesConfig::default();
    let stored = db.config();
    assert_eq!(
        stored.limits.max_collections,
        default.limits.max_collections
    );
    assert_eq!(stored.limits.max_dimensions, default.limits.max_dimensions);
    assert_eq!(stored.wal_batch.enabled, default.wal_batch.enabled);
    assert_eq!(
        stored.wal_batch.max_batch_size,
        default.wal_batch.max_batch_size
    );
}

#[test]
fn test_database_open_with_config_preserves_custom_fields() {
    use crate::config::{LimitsConfig, VelesConfig, WalBatchConfig};

    let dir = tempdir().unwrap();

    let custom = VelesConfig {
        limits: LimitsConfig {
            max_dimensions: 2048,
            max_vectors_per_collection: 50_000_000,
            max_collections: 500,
            max_payload_size: 524_288,
            max_perfect_mode_vectors: 250_000,
        },
        wal_batch: WalBatchConfig {
            enabled: true,
            commit_delay_us: 250,
            max_batch_size: 256,
        },
        ..VelesConfig::default()
    };

    let db = Database::open_with_config(dir.path(), custom).unwrap();
    let stored = db.config();

    assert_eq!(stored.limits.max_dimensions, 2048);
    assert_eq!(stored.limits.max_collections, 500);
    assert_eq!(stored.limits.max_payload_size, 524_288);
    assert!(stored.wal_batch.enabled);
    assert_eq!(stored.wal_batch.commit_delay_us, 250);
    assert_eq!(stored.wal_batch.max_batch_size, 256);
}

#[test]
fn test_database_config_arc_shares_same_instance() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();

    // `config_arc` must hand out clones of the same `Arc`, not
    // deep-clone the inner struct. Sub-systems (background index
    // builders, async reindex managers) rely on this so that config
    // updates propagate without forcing a refcount traversal of the
    // whole database.
    let a = db.config_arc();
    let b = db.config_arc();
    assert!(std::sync::Arc::ptr_eq(&a, &b));
    // And the underlying pointer is the same as the `config()` borrow.
    let r: *const crate::config::VelesConfig = db.config();
    let a_ptr: *const crate::config::VelesConfig = std::sync::Arc::as_ptr(&a);
    assert!(std::ptr::eq(a_ptr, r));
}

// =========================================================================
// LimitsConfig enforcement — Wave 3 Commit 7
//
// Runtime gates that read from `database.config().limits` and refuse
// operations that would exceed a user-supplied ceiling. These tests
// anchor each gate with nominal, edge, and negative coverage so later
// refactors cannot silently relax the enforcement.
// =========================================================================

#[test]
fn test_max_collections_limit_refuses_excess_with_guard_rail_error() {
    use crate::config::{LimitsConfig, VelesConfig};

    let dir = tempdir().unwrap();
    let config = VelesConfig {
        limits: LimitsConfig {
            max_collections: 2,
            ..LimitsConfig::default()
        },
        ..VelesConfig::default()
    };
    let db = Database::open_with_config(dir.path(), config).unwrap();

    db.create_collection("one", 4, DistanceMetric::Cosine)
        .unwrap();
    db.create_collection("two", 4, DistanceMetric::Cosine)
        .unwrap();

    // The third creation must fail with a guard-rail violation carrying
    // the current/cap ratio in the message — that string is the contract
    // for any client-side parser that wants to surface "raise the cap"
    // guidance to the end user.
    let err = db
        .create_collection("three", 4, DistanceMetric::Cosine)
        .unwrap_err();
    match err {
        Error::GuardRail(msg) => {
            assert!(msg.contains("max_collections"));
            assert!(msg.contains("2 / 2"));
        }
        other => panic!("expected GuardRail error, got {other:?}"),
    }
}

#[test]
fn test_max_collections_limit_counts_across_all_registries() {
    use crate::config::{LimitsConfig, VelesConfig};

    let dir = tempdir().unwrap();
    let config = VelesConfig {
        limits: LimitsConfig {
            max_collections: 3,
            ..LimitsConfig::default()
        },
        ..VelesConfig::default()
    };
    let db = Database::open_with_config(dir.path(), config).unwrap();

    // Mix vector + graph + metadata collections — the limit is
    // tenant-wide, not per-type.
    db.create_collection("v1", 4, DistanceMetric::Cosine)
        .unwrap();
    db.create_graph_collection("g1", crate::collection::GraphSchema::new())
        .unwrap();
    db.create_metadata_collection("m1").unwrap();

    // Fourth collection of any kind must be refused.
    let err = db.create_metadata_collection("m2").unwrap_err();
    assert!(matches!(err, Error::GuardRail(_)));
}

#[test]
fn test_max_dimensions_limit_refuses_oversize_vector() {
    use crate::config::{LimitsConfig, VelesConfig};

    let dir = tempdir().unwrap();
    let config = VelesConfig {
        limits: LimitsConfig {
            max_dimensions: 512,
            ..LimitsConfig::default()
        },
        ..VelesConfig::default()
    };
    let db = Database::open_with_config(dir.path(), config).unwrap();

    // Exactly at the cap is accepted (inclusive boundary).
    db.create_collection("boundary", 512, DistanceMetric::Cosine)
        .unwrap();

    // One above the cap is refused with a guard-rail error.
    let err = db
        .create_vector_collection_with_options(
            "too_big",
            513,
            DistanceMetric::Cosine,
            StorageMode::Full,
        )
        .unwrap_err();
    match err {
        Error::GuardRail(msg) => {
            assert!(msg.contains("513"));
            assert!(msg.contains("512"));
            assert!(msg.contains("max_dimensions"));
        }
        other => panic!("expected GuardRail error, got {other:?}"),
    }
}

#[test]
fn test_max_dimensions_limit_applies_to_graph_with_embeddings() {
    use crate::config::{LimitsConfig, VelesConfig};

    let dir = tempdir().unwrap();
    let config = VelesConfig {
        limits: LimitsConfig {
            max_dimensions: 128,
            ..LimitsConfig::default()
        },
        ..VelesConfig::default()
    };
    let db = Database::open_with_config(dir.path(), config).unwrap();

    // Graph WITHOUT embeddings is always accepted — dimension 0 bypasses
    // the gate so metadata-only graphs are unaffected.
    db.create_graph_collection("plain_graph", crate::collection::GraphSchema::new())
        .unwrap();

    // Graph WITH embeddings must obey the same dimension cap as vector
    // collections, because the embeddings live in the same HNSW index.
    let err = db
        .create_graph_collection_with_embeddings(
            "embed_graph",
            crate::collection::GraphSchema::new(),
            256,
            DistanceMetric::Cosine,
        )
        .unwrap_err();
    assert!(matches!(err, Error::GuardRail(_)));
}

#[test]
fn test_limits_config_default_accepts_common_embedding_dims() {
    // Regression guard: the default LimitsConfig must accept every
    // dimension used by popular embedding models without any user
    // configuration. If someone tightens the default too much, this
    // test catches the silent breakage immediately.
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();

    for (name, dim) in [
        ("minilm", 384),
        ("bert", 768),
        ("openai_small", 1536),
        ("openai_large", 3072),
    ] {
        db.create_collection(name, dim, DistanceMetric::Cosine)
            .unwrap_or_else(|e| panic!("default limits should accept {name} ({dim}-d), got {e:?}"));
    }
}

#[test]
fn test_dimension_zero_is_exempt_from_limits_gate() {
    // Metadata-only collections pass dimension=0 down the same
    // pipeline — the dimension-limit gate must NOT reject them.
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();

    db.create_metadata_collection("meta").unwrap();
    assert_eq!(db.list_collections(), vec!["meta"]);
}

#[test]
fn test_raw_bulk_upsert_enforces_vector_count_cap() {
    use crate::config::{LimitsConfig, VelesConfig};

    let dir = tempdir().unwrap();
    let config = VelesConfig {
        limits: LimitsConfig {
            max_vectors_per_collection: 2,
            ..LimitsConfig::default()
        },
        ..VelesConfig::default()
    };
    let db = Database::open_with_config(dir.path(), config).unwrap();
    db.create_collection("v", 4, DistanceMetric::Cosine)
        .unwrap();
    let coll = db.get_vector_collection("v").unwrap();

    // Five vectors via the zero-copy raw path: the cap of 2 must reject it,
    // so the dominant SDK/REST bulk surface cannot bypass the limit.
    let ids = [1u64, 2, 3, 4, 5];
    let vectors: Vec<f32> = (0..ids.len()).flat_map(|_| [0.1, 0.2, 0.3, 0.4]).collect();
    let err = coll
        .upsert_bulk_from_raw(&vectors, &ids, 4, None)
        .unwrap_err();
    assert!(matches!(err, Error::GuardRail(_)), "got {err:?}");
}

#[test]
fn test_raw_bulk_upsert_enforces_payload_size_cap() {
    use crate::config::{LimitsConfig, VelesConfig};

    let dir = tempdir().unwrap();
    let config = VelesConfig {
        limits: LimitsConfig {
            max_payload_size: 16,
            ..LimitsConfig::default()
        },
        ..VelesConfig::default()
    };
    let db = Database::open_with_config(dir.path(), config).unwrap();
    db.create_collection("v", 4, DistanceMetric::Cosine)
        .unwrap();
    let coll = db.get_vector_collection("v").unwrap();

    let ids = [1u64];
    let vectors = [0.1_f32, 0.2, 0.3, 0.4];
    let big = serde_json::json!({ "text": "x".repeat(64) });
    let payloads = [Some(big)];
    let err = coll
        .upsert_bulk_from_raw(&vectors, &ids, 4, Some(&payloads))
        .unwrap_err();
    assert!(matches!(err, Error::GuardRail(_)), "got {err:?}");
}

#[test]
fn test_graph_node_payload_enforces_payload_size_cap() {
    use crate::config::{LimitsConfig, VelesConfig};

    let dir = tempdir().unwrap();
    let config = VelesConfig {
        limits: LimitsConfig {
            max_payload_size: 16,
            ..LimitsConfig::default()
        },
        ..VelesConfig::default()
    };
    let db = Database::open_with_config(dir.path(), config).unwrap();
    db.create_graph_collection("g", crate::collection::GraphSchema::new())
        .unwrap();
    let graph = db.get_graph_collection("g").unwrap();

    // Vector-less node write routes through store_node_payload, which the
    // gate must cover — an oversized payload is rejected.
    let big = serde_json::json!({ "text": "x".repeat(64) });
    let err = graph.upsert_node_payload(1, &big).unwrap_err();
    assert!(matches!(err, Error::GuardRail(_)), "got {err:?}");

    // A small payload still succeeds on the same path.
    let small = serde_json::json!({ "k": 1 });
    graph.upsert_node_payload(2, &small).unwrap();
}
