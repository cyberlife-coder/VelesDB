//! BDD tests for the collection-type-migration feature.
//!
//! Validates `AnyCollection` enum dispatch, `Database::get_any_collection`,
//! persistence round-trips, and typed registry integrity after the v2.0.0
//! migration from the deprecated `Collection` god-object.

use std::collections::HashMap;

use serde_json::json;
use velesdb_core::{AnyCollection, Database, DistanceMetric, Point};

use super::helpers::{create_test_db, execute_sql};

// =========================================================================
// Helpers
// =========================================================================

/// Creates all 3 collection types in the given database.
fn setup_all_three_types(db: &Database) {
    execute_sql(
        db,
        "CREATE COLLECTION vectors (dimension = 4, metric = 'cosine');",
    )
    .expect("test: CREATE vectors");

    execute_sql(
        db,
        "CREATE GRAPH COLLECTION graphs (dimension = 4, metric = 'cosine') SCHEMALESS;",
    )
    .expect("test: CREATE graphs");

    execute_sql(db, "CREATE METADATA COLLECTION meta;").expect("test: CREATE meta");
}

// =========================================================================
// Scenario 1: AnyCollection::Vector dispatch
// =========================================================================

/// GIVEN a Database with a vector collection
/// WHEN get_any_collection is called
/// THEN it returns AnyCollection::Vector and config() returns correct dimension/metric.
#[test]
fn test_any_collection_vector_dispatch_returns_correct_variant_and_config() {
    let (_dir, db) = create_test_db();

    execute_sql(
        &db,
        "CREATE COLLECTION docs (dimension = 4, metric = 'cosine');",
    )
    .expect("test: CREATE docs");

    let any = db
        .get_any_collection("docs")
        .expect("test: get_any_collection should return Some");

    assert!(
        matches!(any, AnyCollection::Vector(_)),
        "Should be AnyCollection::Vector"
    );

    let cfg = any.config();
    assert_eq!(cfg.dimension, 4);
    assert_eq!(cfg.metric, DistanceMetric::Cosine);
}

// =========================================================================
// Scenario 2: AnyCollection::Graph dispatch
// =========================================================================

/// GIVEN a Database with a graph collection
/// WHEN get_any_collection is called
/// THEN it returns AnyCollection::Graph.
#[test]
fn test_any_collection_graph_dispatch_returns_graph_variant() {
    let (_dir, db) = create_test_db();

    execute_sql(
        &db,
        "CREATE GRAPH COLLECTION kg (dimension = 4, metric = 'cosine') SCHEMALESS;",
    )
    .expect("test: CREATE kg");

    let any = db
        .get_any_collection("kg")
        .expect("test: get_any_collection should return Some");

    assert!(
        matches!(any, AnyCollection::Graph(_)),
        "Should be AnyCollection::Graph"
    );
}

// =========================================================================
// Scenario 3: AnyCollection::Metadata dispatch
// =========================================================================

/// GIVEN a Database with a metadata collection
/// WHEN get_any_collection is called
/// THEN it returns AnyCollection::Metadata.
#[test]
fn test_any_collection_metadata_dispatch_returns_metadata_variant() {
    let (_dir, db) = create_test_db();

    execute_sql(&db, "CREATE METADATA COLLECTION tags;").expect("test: CREATE tags");

    let any = db
        .get_any_collection("tags")
        .expect("test: get_any_collection should return Some");

    assert!(
        matches!(any, AnyCollection::Metadata(_)),
        "Should be AnyCollection::Metadata"
    );
    assert!(any.is_metadata_only());
}

// =========================================================================
// Scenario 4: AnyCollection execute_query_str
// =========================================================================

/// GIVEN a vector collection with points
/// WHEN execute_query_str("SELECT * FROM col LIMIT 5") is called on AnyCollection
/// THEN it returns results.
#[test]
fn test_any_collection_execute_query_str_returns_results() {
    let (_dir, db) = create_test_db();

    execute_sql(
        &db,
        "CREATE COLLECTION searchable (dimension = 4, metric = 'cosine');",
    )
    .expect("test: CREATE searchable");

    let vc = db
        .get_vector_collection("searchable")
        .expect("test: get vector collection");
    vc.upsert(vec![
        Point::new(1, vec![1.0, 0.0, 0.0, 0.0], Some(json!({"tag": "a"}))),
        Point::new(2, vec![0.0, 1.0, 0.0, 0.0], Some(json!({"tag": "b"}))),
        Point::new(3, vec![0.0, 0.0, 1.0, 0.0], Some(json!({"tag": "c"}))),
    ])
    .expect("test: upsert points");

    let any = db
        .get_any_collection("searchable")
        .expect("test: get_any_collection");

    let results = any
        .execute_query_str("SELECT * FROM searchable LIMIT 5", &HashMap::new())
        .expect("test: execute_query_str should succeed");

    assert_eq!(results.len(), 3, "Should return all 3 points");
}

// =========================================================================
// Scenario 5: AnyCollection flush
// =========================================================================

/// GIVEN a collection
/// WHEN flush() is called on AnyCollection
/// THEN it succeeds without error.
#[test]
fn test_any_collection_flush_succeeds() {
    let (_dir, db) = create_test_db();

    execute_sql(
        &db,
        "CREATE COLLECTION flushable (dimension = 4, metric = 'cosine');",
    )
    .expect("test: CREATE flushable");

    let any = db
        .get_any_collection("flushable")
        .expect("test: get_any_collection");

    any.flush().expect("test: flush should succeed");
}

// =========================================================================
// Scenario 6: Database no legacy registry — multiple types coexist
// =========================================================================

/// GIVEN a Database
/// WHEN multiple collections of different types are created
/// THEN list_collections returns all names and get_any_collection returns the correct variant for each.
#[test]
fn test_database_multiple_types_list_and_dispatch() {
    let (_dir, db) = create_test_db();
    setup_all_three_types(&db);

    let names = db.list_collections();
    assert!(names.contains(&"vectors".to_string()));
    assert!(names.contains(&"graphs".to_string()));
    assert!(names.contains(&"meta".to_string()));

    assert!(matches!(
        db.get_any_collection("vectors"),
        Some(AnyCollection::Vector(_))
    ));
    assert!(matches!(
        db.get_any_collection("graphs"),
        Some(AnyCollection::Graph(_))
    ));
    assert!(matches!(
        db.get_any_collection("meta"),
        Some(AnyCollection::Metadata(_))
    ));
}

// =========================================================================
// Scenario 7: Persistence round-trip
// =========================================================================

/// GIVEN collections of all 3 types
/// WHEN the database is closed and reopened
/// THEN all collections are present with correct types and configs.
#[test]
fn test_persistence_round_trip_all_types_survive_reopen() {
    let dir = tempfile::tempdir().expect("test: create temp dir");

    // Phase 1: create collections and insert data
    {
        let db = Database::open(dir.path()).expect("test: open db");
        setup_all_three_types(&db);

        let vc = db
            .get_vector_collection("vectors")
            .expect("test: get vectors");
        vc.upsert(vec![Point::new(
            1,
            vec![1.0, 0.0, 0.0, 0.0],
            Some(json!({"k": "v"})),
        )])
        .expect("test: upsert");

        let mc = db.get_metadata_collection("meta").expect("test: get meta");
        mc.upsert(vec![Point::metadata_only(10, json!({"x": 1}))])
            .expect("test: upsert meta");

        // Flush all to ensure persistence
        db.get_any_collection("vectors")
            .unwrap()
            .flush()
            .expect("test: flush vectors");
        db.get_any_collection("graphs")
            .unwrap()
            .flush()
            .expect("test: flush graphs");
        db.get_any_collection("meta")
            .unwrap()
            .flush()
            .expect("test: flush meta");
    }

    // Phase 2: reopen and verify
    {
        let db = Database::open(dir.path()).expect("test: reopen db");

        let names = db.list_collections();
        assert!(
            names.contains(&"vectors".to_string()),
            "vectors should persist"
        );
        assert!(
            names.contains(&"graphs".to_string()),
            "graphs should persist"
        );
        assert!(names.contains(&"meta".to_string()), "meta should persist");

        assert!(
            matches!(
                db.get_any_collection("vectors"),
                Some(AnyCollection::Vector(_))
            ),
            "vectors should reopen as Vector variant"
        );
        assert!(
            matches!(
                db.get_any_collection("graphs"),
                Some(AnyCollection::Graph(_))
            ),
            "graphs should reopen as Graph variant"
        );
        assert!(
            matches!(
                db.get_any_collection("meta"),
                Some(AnyCollection::Metadata(_))
            ),
            "meta should reopen as Metadata variant"
        );

        // Verify config survived
        let any_vec = db.get_any_collection("vectors").unwrap();
        assert_eq!(any_vec.config().dimension, 4);
        assert_eq!(any_vec.config().metric, DistanceMetric::Cosine);
    }
}

// =========================================================================
// Scenario 8: Delete removes from typed registry
// =========================================================================

/// GIVEN a vector collection
/// WHEN delete_collection is called
/// THEN get_any_collection returns None and get_vector_collection returns None.
#[test]
fn test_delete_removes_from_typed_registry() {
    let (_dir, db) = create_test_db();

    execute_sql(
        &db,
        "CREATE COLLECTION doomed (dimension = 4, metric = 'cosine');",
    )
    .expect("test: CREATE doomed");

    assert!(db.get_any_collection("doomed").is_some());
    assert!(db.get_vector_collection("doomed").is_some());

    db.delete_collection("doomed")
        .expect("test: delete should succeed");

    assert!(
        db.get_any_collection("doomed").is_none(),
        "get_any_collection should return None after delete"
    );
    assert!(
        db.get_vector_collection("doomed").is_none(),
        "get_vector_collection should return None after delete"
    );
}

// =========================================================================
// Edge Case 9: get_any_collection nonexistent
// =========================================================================

/// GIVEN an empty Database
/// WHEN get_any_collection("nonexistent") is called
/// THEN it returns None.
#[test]
fn test_get_any_collection_nonexistent_returns_none() {
    let (_dir, db) = create_test_db();

    assert!(
        db.get_any_collection("nonexistent").is_none(),
        "Nonexistent collection should return None"
    );
}

// =========================================================================
// Edge Case 10: get_any_collection after delete
// =========================================================================

/// GIVEN a collection that was created then deleted
/// WHEN get_any_collection is called
/// THEN it returns None.
#[test]
fn test_get_any_collection_after_delete_returns_none() {
    let (_dir, db) = create_test_db();

    execute_sql(&db, "CREATE METADATA COLLECTION temp;").expect("test: CREATE temp");
    assert!(db.get_any_collection("temp").is_some());

    db.delete_collection("temp")
        .expect("test: delete should succeed");

    assert!(
        db.get_any_collection("temp").is_none(),
        "Deleted collection should return None"
    );
}

// =========================================================================
// Edge Case 11: AnyCollection is_empty on empty collection
// =========================================================================

/// GIVEN a newly created collection with no points
/// WHEN is_empty() is called on AnyCollection
/// THEN it returns true.
#[test]
fn test_any_collection_is_empty_on_fresh_collection() {
    let (_dir, db) = create_test_db();

    execute_sql(
        &db,
        "CREATE COLLECTION empty_vec (dimension = 4, metric = 'cosine');",
    )
    .expect("test: CREATE empty_vec");

    let any = db
        .get_any_collection("empty_vec")
        .expect("test: get_any_collection");

    assert!(any.is_empty(), "Fresh collection should be empty");
    assert_eq!(any.point_count(), 0);
}

// =========================================================================
// Edge Case 12: AnyCollection point_count after upsert
// =========================================================================

/// GIVEN a vector collection
/// WHEN points are upserted and point_count() is called on AnyCollection
/// THEN it returns the correct count.
#[test]
fn test_any_collection_point_count_after_upsert() {
    let (_dir, db) = create_test_db();

    execute_sql(
        &db,
        "CREATE COLLECTION counted (dimension = 4, metric = 'cosine');",
    )
    .expect("test: CREATE counted");

    let vc = db
        .get_vector_collection("counted")
        .expect("test: get vector collection");
    vc.upsert(vec![
        Point::new(1, vec![1.0, 0.0, 0.0, 0.0], None),
        Point::new(2, vec![0.0, 1.0, 0.0, 0.0], None),
        Point::new(3, vec![0.0, 0.0, 1.0, 0.0], None),
    ])
    .expect("test: upsert 3 points");

    // Re-fetch via AnyCollection to get fresh config
    let any = db
        .get_any_collection("counted")
        .expect("test: get_any_collection");

    assert_eq!(any.point_count(), 3, "point_count should be 3 after upsert");
    assert!(!any.is_empty());
}

// =========================================================================
// Edge Case 13: Mixed collection types coexist with same prefix
// =========================================================================

/// GIVEN vector, graph, and metadata collections with the same prefix but different names
/// WHEN all are queried
/// THEN each returns the correct type.
#[test]
fn test_mixed_types_same_prefix_coexist() {
    let (_dir, db) = create_test_db();

    execute_sql(
        &db,
        "CREATE COLLECTION data_vectors (dimension = 4, metric = 'cosine');",
    )
    .expect("test: CREATE data_vectors");
    execute_sql(
        &db,
        "CREATE GRAPH COLLECTION data_graph (dimension = 4, metric = 'cosine') SCHEMALESS;",
    )
    .expect("test: CREATE data_graph");
    execute_sql(&db, "CREATE METADATA COLLECTION data_meta;").expect("test: CREATE data_meta");

    assert!(matches!(
        db.get_any_collection("data_vectors"),
        Some(AnyCollection::Vector(_))
    ));
    assert!(matches!(
        db.get_any_collection("data_graph"),
        Some(AnyCollection::Graph(_))
    ));
    assert!(matches!(
        db.get_any_collection("data_meta"),
        Some(AnyCollection::Metadata(_))
    ));
}

// =========================================================================
// Edge Case 14: AnyCollection name() matches creation name
// =========================================================================

/// GIVEN a collection created with name "test_abc"
/// WHEN name() is called on AnyCollection
/// THEN it returns "test_abc".
#[test]
fn test_any_collection_name_matches_creation_name() {
    let (_dir, db) = create_test_db();

    execute_sql(
        &db,
        "CREATE COLLECTION test_abc (dimension = 4, metric = 'cosine');",
    )
    .expect("test: CREATE test_abc");

    let any = db
        .get_any_collection("test_abc")
        .expect("test: get_any_collection");

    assert_eq!(any.name(), "test_abc");
}
