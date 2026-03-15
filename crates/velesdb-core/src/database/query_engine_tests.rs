#![allow(deprecated)] // Tests use legacy Collection via get_collection().

use super::*;
use crate::point::Point;
use crate::velesql::Parser;
use crate::DistanceMetric;
use tempfile::tempdir;

// =========================================================================
// execute_query end-to-end
// =========================================================================

#[test]
fn test_execute_query_select_all_returns_inserted_points() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    db.create_collection("docs", 4, DistanceMetric::Cosine)
        .unwrap();

    let coll = db.get_collection("docs").unwrap();
    coll.upsert(vec![
        Point::new(
            1,
            vec![1.0, 0.0, 0.0, 0.0],
            Some(serde_json::json!({"title": "alpha"})),
        ),
        Point::new(
            2,
            vec![0.0, 1.0, 0.0, 0.0],
            Some(serde_json::json!({"title": "beta"})),
        ),
    ])
    .unwrap();

    let query = Parser::parse("SELECT * FROM docs").unwrap();
    let results = db
        .execute_query(&query, &std::collections::HashMap::new())
        .unwrap();

    assert_eq!(results.len(), 2);
}

#[test]
fn test_execute_query_with_limit() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    db.create_collection("items", 4, DistanceMetric::Cosine)
        .unwrap();

    let coll = db.get_collection("items").unwrap();
    let points: Vec<Point> = (1..=5)
        .map(|i| {
            #[allow(clippy::cast_precision_loss)]
            let v = vec![i as f32, 0.0, 0.0, 0.0];
            Point::new(i, v, Some(serde_json::json!({})))
        })
        .collect();
    coll.upsert(points).unwrap();

    let query = Parser::parse("SELECT * FROM items LIMIT 2").unwrap();
    let results = db
        .execute_query(&query, &std::collections::HashMap::new())
        .unwrap();

    assert!(results.len() <= 2);
}

#[test]
fn test_execute_query_nonexistent_collection_returns_error() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();

    let query = Parser::parse("SELECT * FROM ghost").unwrap();
    let err = db
        .execute_query(&query, &std::collections::HashMap::new())
        .unwrap_err();

    assert!(matches!(err, crate::Error::CollectionNotFound(_)));
}

#[test]
fn test_execute_query_invalid_syntax_returns_error() {
    let result = Parser::parse("SELECTT * FROMM nothing");
    assert!(result.is_err());
}

// =========================================================================
// explain_query
// =========================================================================

#[test]
fn test_explain_query_returns_valid_plan() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    db.create_collection("plans", 4, DistanceMetric::Cosine)
        .unwrap();

    let query = Parser::parse("SELECT * FROM plans").unwrap();
    let plan = db.explain_query(&query).unwrap();

    // First call is a cache miss.
    assert_eq!(plan.cache_hit, Some(false));
    assert_eq!(plan.plan_reuse_count, Some(0));
}

#[test]
fn test_explain_query_cache_hit_after_execute() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    db.create_collection("cached", 4, DistanceMetric::Cosine)
        .unwrap();

    let query = Parser::parse("SELECT * FROM cached").unwrap();

    // execute_query populates the cache on miss.
    db.execute_query(&query, &std::collections::HashMap::new())
        .unwrap();

    // explain_query should now report a cache hit.
    let plan = db.explain_query(&query).unwrap();
    assert_eq!(plan.cache_hit, Some(true));
}

// =========================================================================
// DML: INSERT / UPDATE via execute_query
// =========================================================================

#[test]
fn test_execute_query_insert_into_metadata_collection() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    db.create_collection_typed("items", &crate::CollectionType::MetadataOnly)
        .unwrap();

    let query =
        Parser::parse("INSERT INTO items (id, tag, score) VALUES (1, 'hello', 42.0)").unwrap();
    let results = db
        .execute_query(&query, &std::collections::HashMap::new())
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].point.id, 1);
    let payload = results[0].point.payload.as_ref().unwrap();
    assert_eq!(payload["tag"], serde_json::json!("hello"));
}

#[test]
fn test_execute_query_update_modifies_payload() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    db.create_collection_typed("items", &crate::CollectionType::MetadataOnly)
        .unwrap();

    let coll = db.get_collection("items").unwrap();
    coll.upsert_metadata(vec![Point::metadata_only(
        1,
        serde_json::json!({"status": "draft", "count": 0}),
    )])
    .unwrap();

    let query = Parser::parse("UPDATE items SET status = 'published' WHERE id = 1").unwrap();
    let results = db
        .execute_query(&query, &std::collections::HashMap::new())
        .unwrap();
    assert_eq!(results.len(), 1);

    let updated = coll.get(&[1]).into_iter().flatten().next().unwrap();
    let payload = updated.payload.unwrap();
    assert_eq!(payload["status"], serde_json::json!("published"));
    // Unmodified fields are preserved.
    assert_eq!(payload["count"], serde_json::json!(0));
}

// =========================================================================
// Schema version interaction with plan cache
// =========================================================================

#[test]
fn test_schema_version_increments_on_create_and_delete() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    let v0 = db.schema_version();

    db.create_collection("a", 4, DistanceMetric::Cosine)
        .unwrap();
    let v1 = db.schema_version();
    assert!(v1 > v0, "schema_version should increment after create");

    db.delete_collection("a").unwrap();
    let v2 = db.schema_version();
    assert!(v2 > v1, "schema_version should increment after delete");
}
