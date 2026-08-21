//! Tests for the JOIN-side `ColumnStore` cache (CACHE-03).
//!
//! The cache contract: a cached store is reused only while the collection's
//! `(schema_version, write_generation)` stamp is unchanged; any mutation, a
//! drop/recreate under the same name, or the presence of TTL points must
//! force a rebuild (TTL points are never cached at all, because lazy expiry
//! does not bump `write_generation`).

use super::*;
use crate::point::Point;
use crate::DistanceMetric;
use tempfile::tempdir;

fn open_db_with_collection(dir: &std::path::Path) -> Database {
    let db = Database::open(dir).unwrap();
    db.create_collection("orders", 4, DistanceMetric::Cosine)
        .unwrap();
    upsert_order(&db, 1, "a");
    db
}

fn upsert_order(db: &Database, id: u64, sku: &str) {
    let coll = db.resolve_collection("orders").unwrap();
    coll.upsert([Point::new(
        id,
        vec![0.0, 1.0, 0.0, 0.0],
        Some(serde_json::json!({ "sku": sku })),
    )])
    .unwrap();
}

fn cached_store(db: &Database) -> std::sync::Arc<crate::column_store::ColumnStore> {
    let coll = db.resolve_collection("orders").unwrap();
    db.cached_join_column_store("orders", &coll).unwrap()
}

#[test]
fn unchanged_collection_reuses_the_cached_store() {
    let dir = tempdir().unwrap();
    let db = open_db_with_collection(dir.path());

    let first = cached_store(&db);
    let second = cached_store(&db);
    assert!(
        std::sync::Arc::ptr_eq(&first, &second),
        "two joins without any mutation must share the same cached store"
    );
}

#[test]
fn upsert_invalidates_the_cached_store() {
    let dir = tempdir().unwrap();
    let db = open_db_with_collection(dir.path());

    let first = cached_store(&db);
    upsert_order(&db, 2, "b");
    let second = cached_store(&db);
    assert!(
        !std::sync::Arc::ptr_eq(&first, &second),
        "an upsert bumps write_generation and must force a rebuild"
    );
    assert!(second.primary_index.contains_key(&2));
}

#[test]
fn delete_and_recreate_never_serves_the_old_store() {
    let dir = tempdir().unwrap();
    let db = open_db_with_collection(dir.path());

    let first = cached_store(&db);
    db.delete_collection("orders").unwrap();
    assert!(
        !db.join_store_cache.read().contains_key("orders"),
        "delete_collection must purge the cache entry eagerly"
    );

    db.create_collection("orders", 4, DistanceMetric::Cosine)
        .unwrap();
    upsert_order(&db, 9, "z");
    let second = cached_store(&db);
    assert!(
        !std::sync::Arc::ptr_eq(&first, &second),
        "a recreated collection must never reuse the deleted collection's store"
    );
    assert!(second.primary_index.contains_key(&9));
    assert!(!second.primary_index.contains_key(&1));
}

#[test]
fn ttl_points_disable_caching() {
    let dir = tempdir().unwrap();
    let db = open_db_with_collection(dir.path());

    let coll = db.resolve_collection("orders").unwrap();
    let far_future = 4_000_000_000_u64;
    coll.upsert([Point::new(
        2,
        vec![1.0, 0.0, 0.0, 0.0],
        Some(serde_json::json!({
            "sku": "ttl",
            crate::collection::expiry::EXPIRES_AT_KEY: far_future,
        })),
    )])
    .unwrap();

    let first = cached_store(&db);
    let second = cached_store(&db);
    assert!(
        !std::sync::Arc::ptr_eq(&first, &second),
        "a collection carrying TTL points must never be served from cache"
    );
    assert!(
        !db.join_store_cache.read().contains_key("orders"),
        "TTL-carrying builds must not populate the cache"
    );
}

// =========================================================================
// Indexed non-PK JOIN — served from a secondary index, no ColumnStore
// =========================================================================

fn run_sql(db: &Database, sql: &str) -> crate::Result<Vec<crate::SearchResult>> {
    let query =
        crate::velesql::Parser::parse(sql).map_err(|e| crate::Error::Query(e.to_string()))?;
    db.execute_query(&query, &std::collections::HashMap::new())
}

/// `products(id, sku) JOIN inventory ON inventory.sku = products.sku`, with
/// a secondary index on `inventory.sku`. One product sku matches TWO inventory
/// rows — a shape the PK-only path cannot express.
fn seed_indexed_join_fixtures(db: &Database) {
    db.create_collection("products", 4, DistanceMetric::Cosine)
        .unwrap();
    db.create_collection("inventory", 4, DistanceMetric::Cosine)
        .unwrap();
    let products = db.resolve_collection("products").unwrap();
    products
        .upsert([
            Point::new(
                1,
                vec![1.0, 0.0, 0.0, 0.0],
                Some(serde_json::json!({"sku": "a"})),
            ),
            Point::new(
                2,
                vec![0.0, 1.0, 0.0, 0.0],
                Some(serde_json::json!({"sku": "zz"})),
            ),
        ])
        .unwrap();
    let inventory = db.resolve_collection("inventory").unwrap();
    inventory
        .upsert([
            Point::new(
                10,
                vec![0.0; 4],
                Some(serde_json::json!({"sku": "a", "wh": "paris"})),
            ),
            Point::new(
                11,
                vec![0.0; 4],
                Some(serde_json::json!({"sku": "a", "wh": "lyon"})),
            ),
            Point::new(
                12,
                vec![0.0; 4],
                Some(serde_json::json!({"sku": "b", "wh": "nice"})),
            ),
        ])
        .unwrap();
    inventory.create_index("sku").unwrap();
}

#[test]
fn indexed_inner_join_emits_one_row_per_match() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    seed_indexed_join_fixtures(&db);

    let rows = run_sql(
        &db,
        "SELECT * FROM products JOIN inventory ON inventory.sku = products.sku LIMIT 10",
    )
    .unwrap();
    // sku "a" matches two inventory rows; sku "zz" matches none (INNER drops it).
    assert_eq!(rows.len(), 2, "one merged row per indexed match");
    let mut warehouses: Vec<String> = rows
        .iter()
        .filter_map(|r| {
            r.point
                .payload
                .as_ref()?
                .get("wh")?
                .as_str()
                .map(String::from)
        })
        .collect();
    warehouses.sort();
    assert_eq!(warehouses, ["lyon", "paris"]);
}

#[test]
fn indexed_left_join_keeps_unmatched_left_rows() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    seed_indexed_join_fixtures(&db);

    let rows = run_sql(
        &db,
        "SELECT * FROM products LEFT JOIN inventory ON inventory.sku = products.sku LIMIT 10",
    )
    .unwrap();
    // Two matches for "a" plus the bare unmatched "zz" row.
    assert_eq!(rows.len(), 3);
    assert!(rows.iter().any(|r| {
        let p = r.point.payload.as_ref().unwrap();
        p.get("sku").and_then(|v| v.as_str()) == Some("zz") && p.get("wh").is_none()
    }));
}

/// The reject contract is untouched: a non-PK join column WITHOUT a
/// secondary index still fails with the actionable primary-key error.
#[test]
fn unindexed_non_pk_join_column_still_rejected() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    seed_indexed_join_fixtures(&db);

    let err = run_sql(
        &db,
        "SELECT * FROM products JOIN inventory ON inventory.wh = products.sku LIMIT 10",
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("requires primary key"),
        "got: {err}"
    );
}
