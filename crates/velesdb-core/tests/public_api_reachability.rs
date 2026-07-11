//! Public-API reachability integration test (Wave 2, downstream Requirement 9.5,
//! core-side obligation).
//!
//! Feature: core-control-plane-boundary, Task 7.2
//!
//! This suite exercises four shared capabilities — VelesQL/JOIN, hybrid BM25
//! search, product quantization, and mmap storage — **strictly through
//! `velesdb-core`'s public API surface**, exactly as a downstream consumer
//! (e.g. `velesdb-premium`) would. No private module is touched: every symbol
//! is reached via `velesdb_core::...` re-exports. The goal is to prove these
//! capabilities are reachable (and behave sanely) for de-duplication, so
//! premium can call core instead of forking it.
//!
//! _Requirements: 9.5 (core-side obligation)_

#![cfg(feature = "persistence")]

use std::collections::HashMap;

use serde_json::json;
use tempfile::TempDir;
use velesdb_core::filter::Filter;
use velesdb_core::quantization::{train_opq, PQCodebook, PQVector, ProductQuantizer};
use velesdb_core::storage::{MmapStorage, VectorStorage};
use velesdb_core::velesql::{Condition, JoinClause, JoinType, Parser, Query};
use velesdb_core::{
    Database, DistanceMetric, DurabilityMode, FusionStrategy, Point, StorageMode, VectorCollection,
};

/// Seeds a `products` vector collection and a matching `inventory` metadata
/// collection so a JOIN on the primary key yields rows.
fn seed_join_dataset(db: &Database) {
    db.create_collection("products", 4, DistanceMetric::Cosine)
        .expect("test: create products");
    let products = db
        .get_vector_collection("products")
        .expect("test: get products");
    products
        .upsert(vec![
            Point::new(
                1,
                vec![1.0, 0.0, 0.0, 0.0],
                Some(json!({"category": "audio"})),
            ),
            Point::new(
                2,
                vec![0.0, 1.0, 0.0, 0.0],
                Some(json!({"category": "input"})),
            ),
            Point::new(
                3,
                vec![0.0, 0.0, 1.0, 0.0],
                Some(json!({"category": "display"})),
            ),
            Point::new(
                4,
                vec![0.0, 0.0, 0.0, 1.0],
                Some(json!({"category": "input"})),
            ),
            Point::new(
                5,
                vec![0.7, 0.7, 0.0, 0.0],
                Some(json!({"category": "audio"})),
            ),
        ])
        .expect("test: upsert products");

    db.create_metadata_collection("inventory")
        .expect("test: create inventory");
    let inventory = db
        .get_metadata_collection("inventory")
        .expect("test: get inventory");
    inventory
        .upsert(vec![
            Point::metadata_only(1, json!({"price": 99.99, "stock": 50})),
            Point::metadata_only(2, json!({"price": 149.99, "stock": 0})),
            Point::metadata_only(3, json!({"price": 399.99, "stock": 12})),
            Point::metadata_only(4, json!({"price": 29.99, "stock": 200})),
            Point::metadata_only(5, json!({"price": 79.99, "stock": 30})),
        ])
        .expect("test: upsert inventory");
}

/// VelesQL/JOIN is reachable and executes via the public `Database` +
/// `velesql` AST surface (`Parser`, `Query`, `Condition`, `JoinClause`,
/// `JoinType`).
#[test]
fn velesql_join_reachable_through_public_database_api() {
    let dir = TempDir::new().expect("test: tempdir");
    let db = Database::open(dir.path()).expect("test: open db");
    seed_join_dataset(&db);

    // A JOIN parses into the public `Query` AST with reachable JoinClause/JoinType.
    let joined: Query = Parser::parse(
        "SELECT * FROM products JOIN inventory ON products.id = inventory.id LIMIT 10",
    )
    .expect("test: parse JOIN");
    let clause: &JoinClause = joined
        .select
        .joins
        .first()
        .expect("test: JOIN clause present");
    assert_eq!(clause.join_type, JoinType::Inner, "default JOIN is INNER");
    assert_eq!(clause.table.as_str(), "inventory");

    // The JOIN executes through the public `execute_query` surface.
    let rows = db
        .execute_query(&joined, &HashMap::new())
        .expect("test: execute JOIN");
    assert_eq!(rows.len(), 5, "all products join their inventory row");

    // The public `velesql::Condition` AST is reachable via a WHERE clause.
    let filtered: Query = Parser::parse("SELECT * FROM products WHERE category = 'audio' LIMIT 10")
        .expect("test: parse WHERE");
    let cond: &Condition = filtered
        .select
        .where_clause
        .as_ref()
        .expect("test: WHERE present");
    assert!(
        matches!(cond, Condition::Comparison(_)),
        "WHERE is a comparison condition"
    );
    let audio = db
        .execute_query(&filtered, &HashMap::new())
        .expect("test: execute WHERE");
    assert!(!audio.is_empty(), "audio products exist and are returned");
}

/// Hybrid BM25 search and RRF fusion are reachable via the public
/// `VectorCollection` + `FusionStrategy` surface.
#[test]
fn hybrid_bm25_reachable_through_public_vector_collection_api() {
    let dir = TempDir::new().expect("test: tempdir");
    let coll = VectorCollection::create(
        dir.path().join("docs"),
        "docs",
        4,
        DistanceMetric::Cosine,
        StorageMode::Full,
    )
    .expect("test: create collection");
    coll.upsert(vec![
        Point::new(
            1,
            vec![1.0, 0.0, 0.0, 0.0],
            Some(json!({"text": "rust systems programming"})),
        ),
        Point::new(
            2,
            vec![0.0, 1.0, 0.0, 0.0],
            Some(json!({"text": "chocolate cake recipe"})),
        ),
        Point::new(
            3,
            vec![0.7, 0.3, 0.0, 0.0],
            Some(json!({"text": "rust performance tuning"})),
        ),
    ])
    .expect("test: upsert docs");

    // RRF fusion is reachable through the public `FusionStrategy` surface.
    let rrf = FusionStrategy::rrf_default();
    let fused = rrf
        .fuse(vec![vec![(1u64, 0.9), (3u64, 0.5)]])
        .expect("test: rrf fuse");
    assert!(!fused.is_empty(), "RRF produces fused results");

    // `hybrid_search` is reachable through the public `VectorCollection` API.
    let hybrid = coll
        .hybrid_search(&[1.0, 0.0, 0.0, 0.0], "rust", 3, Some(0.5))
        .expect("test: hybrid search");
    assert!(!hybrid.is_empty(), "hybrid search returns fused results");

    // `hybrid_search_with_filter` is reachable through the public API.
    let filter = Filter::new(velesdb_core::filter::Condition::eq(
        "text",
        "rust systems programming",
    ));
    let filtered = coll
        .hybrid_search_with_filter(&[1.0, 0.0, 0.0, 0.0], "rust", 3, Some(0.5), &filter)
        .expect("test: hybrid filtered search");
    assert!(
        filtered.len() <= hybrid.len(),
        "a metadata filter narrows (never widens) the hybrid result set"
    );
}

/// Deterministic training corpus: 16 vectors of dimension 8.
#[allow(clippy::cast_precision_loss)]
fn pq_training_vectors() -> Vec<Vec<f32>> {
    (0..16u32)
        .map(|i| (0..8u32).map(|j| ((i * 8 + j) as f32).sin()).collect())
        .collect()
}

/// Product quantization is reachable via the public `StorageMode` +
/// `quantization` surface (`ProductQuantizer`, `PQCodebook`, `PQVector`,
/// `train_opq`).
#[test]
fn product_quantization_reachable_through_public_quantization_api() {
    // `StorageMode::ProductQuantization` is reachable and canonical.
    assert_eq!(StorageMode::ProductQuantization.canonical_name(), "pq");
    assert_eq!(
        StorageMode::parse_alias("pq"),
        Some(StorageMode::ProductQuantization)
    );

    let vectors = pq_training_vectors();

    // train / quantize / reconstruct are reachable through the public API.
    let quantizer: ProductQuantizer =
        ProductQuantizer::train(&vectors, 2, 4).expect("test: train PQ");
    let codebook: &PQCodebook = &quantizer.codebook;
    assert_eq!(codebook.num_subspaces, 2);
    assert_eq!(codebook.dimension, 8);

    let encoded: PQVector = quantizer.quantize(&vectors[0]).expect("test: quantize");
    assert_eq!(encoded.codes.len(), 2, "one code per subspace");
    let reconstructed = quantizer.reconstruct(&encoded).expect("test: reconstruct");
    assert_eq!(reconstructed.len(), 8, "reconstruction matches dimension");

    // `train_opq` (OPQ rotation) is reachable and yields a rotation matrix.
    let opq = train_opq(&vectors, 2, 4, true, 2).expect("test: train OPQ");
    assert!(
        opq.rotation.is_some(),
        "OPQ training yields a rotation matrix"
    );
}

/// mmap storage is reachable via the public `storage::MmapStorage` +
/// `DurabilityMode` surface, round-tripping a stored vector.
#[test]
fn mmap_storage_reachable_through_public_storage_api() {
    let dir = TempDir::new().expect("test: tempdir");
    let mut storage = MmapStorage::new_with_durability(
        dir.path().join("vectors.mmap"),
        4,
        DurabilityMode::default(),
    )
    .expect("test: create mmap storage");

    let a = [1.0f32, 2.0, 3.0, 4.0];
    let b = [5.0f32, 6.0, 7.0, 8.0];
    storage.store(1, &a).expect("test: store vec 1");
    storage.store(2, &b).expect("test: store vec 2");
    storage.flush().expect("test: flush");

    assert_eq!(storage.len(), 2, "two vectors stored");
    let got = storage
        .retrieve(1)
        .expect("test: retrieve")
        .expect("test: vec 1 present");
    assert_eq!(got.len(), 4);
    assert!(
        got.iter()
            .zip(a.iter())
            .all(|(x, y)| (x - y).abs() < f32::EPSILON),
        "mmap round-trips the stored vector byte-for-byte"
    );
}
