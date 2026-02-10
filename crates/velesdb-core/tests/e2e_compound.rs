//! E2E integration tests for compound query execution (UNION/INTERSECT/EXCEPT).
//!
//! Validates that `Database::execute_query()` correctly handles set operations
//! across collections through the full VelesQL pipeline.

use std::collections::HashMap;

use serde_json::json;

use velesdb_core::distance::DistanceMetric;
use velesdb_core::{Database, Point};

/// Creates a Database with two collections for compound query scenarios:
/// - "active_docs": IDs {1,2,3} — currently active documents
/// - "archived_docs": IDs {2,3,4,5} — archived documents
///   Overlap on IDs 2 and 3 exercises dedup/intersect/except.
fn setup_compound_db() -> (Database, tempfile::TempDir) {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let db = Database::open(dir.path()).expect("open");

    db.create_collection("active_docs", 4, DistanceMetric::Cosine)
        .expect("create active_docs");
    let active = db.get_collection("active_docs").expect("get active_docs");
    active
        .upsert(vec![
            Point::new(
                1,
                vec![1.0, 0.0, 0.0, 0.0],
                Some(json!({"id": 1, "title": "Rust Guide", "category": "tech", "status": "active"})),
            ),
            Point::new(
                2,
                vec![0.0, 1.0, 0.0, 0.0],
                Some(json!({"id": 2, "title": "Python Intro", "category": "tech", "status": "active"})),
            ),
            Point::new(
                3,
                vec![0.0, 0.0, 1.0, 0.0],
                Some(json!({"id": 3, "title": "AI Primer", "category": "tech", "status": "active"})),
            ),
        ])
        .expect("upsert active_docs");

    db.create_collection("archived_docs", 4, DistanceMetric::Cosine)
        .expect("create archived_docs");
    let archived = db
        .get_collection("archived_docs")
        .expect("get archived_docs");
    archived
        .upsert(vec![
            Point::new(
                2,
                vec![0.0, 1.0, 0.0, 0.0],
                Some(json!({"id": 2, "title": "Python Intro v1", "category": "tech", "status": "archived"})),
            ),
            Point::new(
                3,
                vec![0.0, 0.0, 1.0, 0.0],
                Some(json!({"id": 3, "title": "AI Primer v1", "category": "tech", "status": "archived"})),
            ),
            Point::new(
                4,
                vec![0.0, 0.0, 0.0, 1.0],
                Some(json!({"id": 4, "title": "Old Database Book", "category": "tech", "status": "archived"})),
            ),
            Point::new(
                5,
                vec![0.5, 0.5, 0.0, 0.0],
                Some(json!({"id": 5, "title": "Legacy API Docs", "category": "tech", "status": "archived"})),
            ),
        ])
        .expect("upsert archived_docs");

    (db, dir)
}

// ========== UNION ==========

#[test]
fn test_e2e_union_deduplicates_across_collections() {
    let (db, _dir) = setup_compound_db();

    let query = velesdb_core::velesql::Parser::parse(
        "SELECT * FROM active_docs UNION SELECT * FROM archived_docs",
    )
    .expect("parse");
    let params = HashMap::new();
    let results = db.execute_query(&query, &params).expect("execute");

    // active {1,2,3} ∪ archived {2,3,4,5} → {1,2,3,4,5}
    let ids: std::collections::HashSet<u64> = results.iter().map(|r| r.point.id).collect();
    assert_eq!(
        ids.len(),
        5,
        "UNION should deduplicate: expected 5 unique IDs, got {:?}",
        ids
    );
    for expected_id in 1..=5 {
        assert!(ids.contains(&expected_id), "Missing ID {expected_id}");
    }
}

#[test]
fn test_e2e_union_all_keeps_duplicates() {
    let (db, _dir) = setup_compound_db();

    let query = velesdb_core::velesql::Parser::parse(
        "SELECT * FROM active_docs UNION ALL SELECT * FROM archived_docs",
    )
    .expect("parse");
    let params = HashMap::new();
    let results = db.execute_query(&query, &params).expect("execute");

    // UNION ALL: 3 + 4 = 7 rows (no dedup)
    assert_eq!(
        results.len(),
        7,
        "UNION ALL should keep all rows (3+4=7), got {}",
        results.len()
    );
}

// ========== INTERSECT ==========

#[test]
fn test_e2e_intersect_common_documents() {
    let (db, _dir) = setup_compound_db();

    let query = velesdb_core::velesql::Parser::parse(
        "SELECT * FROM active_docs INTERSECT SELECT * FROM archived_docs",
    )
    .expect("parse");
    let params = HashMap::new();
    let results = db.execute_query(&query, &params).expect("execute");

    // active {1,2,3} ∩ archived {2,3,4,5} → {2,3}
    let ids: std::collections::HashSet<u64> = results.iter().map(|r| r.point.id).collect();
    assert_eq!(
        ids.len(),
        2,
        "INTERSECT should return only common IDs, got {:?}",
        ids
    );
    assert!(ids.contains(&2), "ID 2 should be in intersection");
    assert!(ids.contains(&3), "ID 3 should be in intersection");
}

// ========== EXCEPT ==========

#[test]
fn test_e2e_except_only_in_first_collection() {
    let (db, _dir) = setup_compound_db();

    let query = velesdb_core::velesql::Parser::parse(
        "SELECT * FROM active_docs EXCEPT SELECT * FROM archived_docs",
    )
    .expect("parse");
    let params = HashMap::new();
    let results = db.execute_query(&query, &params).expect("execute");

    // active {1,2,3} - archived {2,3,4,5} → {1}
    assert_eq!(
        results.len(),
        1,
        "EXCEPT should return only IDs in left but not right"
    );
    assert_eq!(
        results[0].point.id, 1,
        "Only ID 1 is exclusive to active_docs"
    );
}

// ========== Same-Collection UNION ==========

#[test]
fn test_e2e_union_same_collection_different_where() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let db = Database::open(dir.path()).expect("open");

    db.create_collection("docs", 4, DistanceMetric::Cosine)
        .expect("create");
    let docs = db.get_collection("docs").expect("get");
    docs.upsert(vec![
        Point::new(1, vec![1.0, 0.0, 0.0, 0.0], Some(json!({"tag": "a"}))),
        Point::new(2, vec![0.0, 1.0, 0.0, 0.0], Some(json!({"tag": "b"}))),
        Point::new(3, vec![0.0, 0.0, 1.0, 0.0], Some(json!({"tag": "a"}))),
        Point::new(4, vec![0.0, 0.0, 0.0, 1.0], Some(json!({"tag": "c"}))),
    ])
    .expect("upsert");

    // UNION of same collection with different WHERE filters
    let query = velesdb_core::velesql::Parser::parse(
        "SELECT * FROM docs WHERE tag = 'a' UNION SELECT * FROM docs WHERE tag = 'b'",
    )
    .expect("parse");
    let params = HashMap::new();
    let results = db.execute_query(&query, &params).expect("execute");

    // tag='a' → {1,3}, tag='b' → {2} → UNION → {1,2,3}
    let ids: std::collections::HashSet<u64> = results.iter().map(|r| r.point.id).collect();
    assert_eq!(
        ids.len(),
        3,
        "UNION on same collection with different WHERE should yield 3 results, got {:?}",
        ids
    );
    assert!(ids.contains(&1));
    assert!(ids.contains(&2));
    assert!(ids.contains(&3));
    assert!(!ids.contains(&4), "tag='c' should not be in results");
}
