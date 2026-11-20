//! Tests for the JOIN-side points cache (CACHE-03).
//!
//! The cache contract: cached points are reused only while the collection's
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

fn cached_store(db: &Database) -> std::sync::Arc<Vec<Point>> {
    let coll = db.resolve_collection("orders").unwrap();
    db.cached_join_points("orders", &coll)
}

fn ids(points: &[Point]) -> Vec<u64> {
    points.iter().map(|p| p.id).collect()
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
    assert_eq!(ids(&second), [1, 2]);
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
    assert_eq!(ids(&second), [9]);
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

/// A non-PK join column WITHOUT a secondary index runs the generic hash
/// join (D014): `products.sku` carries no index, and each inventory row
/// pairs with the product of its sku.
#[test]
fn unindexed_non_pk_join_column_runs_the_hash_join() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    seed_indexed_join_fixtures(&db);

    let rows = run_sql(
        &db,
        "SELECT * FROM inventory JOIN products ON inventory.sku = products.sku \
         ORDER BY inventory.id ASC LIMIT 10",
    )
    .unwrap();
    let pairs: Vec<(u64, Option<u64>)> = rows
        .iter()
        .map(|r| {
            let payload = r.point.payload.as_ref().unwrap();
            (r.point.id, payload["products"]["id"].as_u64())
        })
        .collect();
    assert_eq!(pairs, [(10, Some(1)), (11, Some(1))]);
}

/// An indexed join column whose base value is an array runs the hash join:
/// the secondary index keys scalars only, so its shortcut found no match
/// for an array that the joined point holds as is.
#[test]
fn an_array_join_key_matches_though_the_column_is_indexed() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    seed_indexed_join_fixtures(&db);
    db.resolve_collection("products")
        .unwrap()
        .upsert([Point::new(
            3,
            vec![0.0, 0.0, 1.0, 0.0],
            Some(serde_json::json!({"sku": ["a", "b"]})),
        )])
        .unwrap();
    db.resolve_collection("inventory")
        .unwrap()
        .upsert([Point::new(
            13,
            vec![0.0; 4],
            Some(serde_json::json!({"sku": ["a", "b"], "wh": "metz"})),
        )])
        .unwrap();

    let rows = run_sql(
        &db,
        "SELECT * FROM products JOIN inventory ON products.sku = inventory.sku LIMIT 10",
    )
    .unwrap();
    let mut pairs: Vec<(u64, Option<u64>)> = rows
        .iter()
        .map(|r| {
            let payload = r.point.payload.as_ref().unwrap();
            (r.point.id, payload["inventory"]["id"].as_u64())
        })
        .collect();
    pairs.sort_unstable();
    assert_eq!(pairs, [(1, Some(10)), (1, Some(11)), (3, Some(13))]);
}

// =========================================================================
// Base side of a join — the FROM table, whatever the JOINs are aliased
// =========================================================================

/// An unaliased FROM is read under its table name even when the JOIN is
/// aliased: `from_alias` lists the JOIN alias too, and taking its first
/// entry scoped the base rows under `p`, leaving `inventory.*` unreadable.
#[test]
fn an_unaliased_base_reads_under_its_table_beside_an_aliased_join() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    seed_indexed_join_fixtures(&db);

    let rows = run_sql(
        &db,
        "SELECT * FROM inventory JOIN products AS p ON inventory.sku = p.sku \
         ORDER BY inventory.id ASC LIMIT 10",
    )
    .unwrap();
    let sides: Vec<(Option<u64>, Option<u64>)> = rows
        .iter()
        .map(|r| {
            let payload = r.point.payload.as_ref().unwrap();
            (
                payload["inventory"]["id"].as_u64(),
                payload["p"]["id"].as_u64(),
            )
        })
        .collect();
    assert_eq!(sides, [(Some(10), Some(1)), (Some(11), Some(1))]);
}

/// A predicate on a LEFT JOIN's alias holds on the joined rows. Read as a
/// base-collection name (`from_alias` lists JOIN aliases too), it ran on the
/// base rows, where `p` is absent, so `p.id IS NULL` let every row through.
#[test]
fn an_outer_join_alias_predicate_holds_on_the_joined_rows() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    seed_indexed_join_fixtures(&db);

    let rows = run_sql(
        &db,
        "SELECT * FROM inventory LEFT JOIN products AS p ON inventory.sku = p.sku \
         WHERE p.id IS NULL LIMIT 10",
    )
    .unwrap();
    let ids: Vec<u64> = rows.iter().map(|r| r.point.id).collect();
    assert_eq!(ids, [12], "only the `nice` row has no product");
}

// =========================================================================
// A text MATCH in a JOIN: the side its qualifier names
// =========================================================================

/// `docs` (texts ranking "apple apple apple" (6) above "apple pie" (1) above
/// "Apple banana smoothie" (3)) and `tags`, one row per doc id.
fn seed_text_join_fixtures(db: &Database) {
    db.create_collection("docs", 2, DistanceMetric::Cosine)
        .unwrap();
    db.create_collection("tags", 2, DistanceMetric::Cosine)
        .unwrap();
    let texts = [
        (1, "apple pie"),
        (2, "banana bread"),
        (3, "Apple banana smoothie"),
        (4, "pineapple juice"),
        (5, "cherry tart"),
        (6, "apple apple apple"),
    ];
    #[allow(clippy::cast_precision_loss)]
    let docs = texts.iter().map(|&(id, content)| {
        Point::new(
            id,
            vec![1.0, id as f32],
            Some(serde_json::json!({"content": content})),
        )
    });
    db.resolve_collection("docs").unwrap().upsert(docs).unwrap();
    let tags =
        (1..=6).map(|id| Point::new(id, vec![1.0, 0.0], Some(serde_json::json!({"tag": id}))));
    db.resolve_collection("tags").unwrap().upsert(tags).unwrap();
}

fn result_ids(results: &[crate::SearchResult]) -> Vec<u64> {
    results.iter().map(|r| r.point.id).collect()
}

/// The AND-required MATCH ranks the base rows by BM25 however its column is
/// written — unqualified, or qualified by the base side's name, the table's
/// or its alias — as it does without a JOIN.
#[test]
fn a_base_side_match_ranks_the_joined_rows_however_it_is_qualified() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    seed_text_join_fixtures(&db);

    for sql in [
        "SELECT * FROM docs JOIN tags ON docs.id = tags.id WHERE content MATCH 'apple' LIMIT 2",
        "SELECT * FROM docs JOIN tags ON docs.id = tags.id WHERE docs.content MATCH 'apple' LIMIT 2",
        "SELECT * FROM docs AS d JOIN tags AS t ON d.id = t.id WHERE d.content MATCH 'apple' LIMIT 2",
    ] {
        let rows = run_sql(&db, sql).unwrap_or_else(|e| panic!("{sql}: {e}"));
        assert_eq!(result_ids(&rows), [6, 1], "{sql}");
    }
}

/// A qualifier naming no side of the JOIN — the table name of an aliased
/// FROM — is refused: no text index answers it, and the base's must not.
#[test]
fn a_match_qualified_by_no_side_of_the_join_is_refused() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    seed_text_join_fixtures(&db);

    for sql in [
        "SELECT * FROM docs AS d JOIN tags AS t ON d.id = t.id WHERE docs.content MATCH 'apple' LIMIT 2",
        "SELECT * FROM docs AS d JOIN tags AS t ON d.id = t.id \
         WHERE nowhere.content MATCH 'apple' OR t.tag = 2 LIMIT 2",
    ] {
        let err = run_sql(&db, sql).expect_err(sql);
        assert!(err.to_string().contains("names no side"), "{sql}: {err}");
    }
}

/// A MATCH on a joined side tests that side's hits for the joined row.
#[test]
fn a_joined_side_match_tests_that_sides_hits() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    seed_text_join_fixtures(&db);

    let rows = run_sql(
        &db,
        "SELECT * FROM tags AS t JOIN docs AS d ON t.id = d.id WHERE d.content MATCH 'apple' \
         ORDER BY id LIMIT 10",
    )
    .unwrap();
    assert_eq!(result_ids(&rows), [1, 3, 6]);
}
