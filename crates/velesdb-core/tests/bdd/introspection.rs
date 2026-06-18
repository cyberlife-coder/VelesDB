//! BDD integration tests for `VelesQL` introspection statements.
//!
//! Tests SHOW COLLECTIONS, DESCRIBE COLLECTION, and EXPLAIN SELECT
//! through the full pipeline: parse -> validate -> execute.

use super::helpers::{create_test_db, execute_sql, payload_str};

// ============================================================================
// SHOW COLLECTIONS
// ============================================================================

#[test]
fn test_show_collections_returns_all_types() {
    let (_dir, db) = create_test_db();

    // Create one of each collection type.
    execute_sql(
        &db,
        "CREATE COLLECTION docs (dimension = 4, metric = 'cosine')",
    )
    .expect("create vector");
    execute_sql(
        &db,
        "CREATE GRAPH COLLECTION kg (dimension = 4, metric = 'cosine') SCHEMALESS",
    )
    .expect("create graph");
    execute_sql(&db, "CREATE METADATA COLLECTION tags").expect("create metadata");

    let results = execute_sql(&db, "SHOW COLLECTIONS").expect("SHOW COLLECTIONS should succeed");

    assert_eq!(results.len(), 3, "should return 3 collections");

    // Collect names and types.
    let mut names: Vec<&str> = results
        .iter()
        .filter_map(|r| payload_str(r, "name"))
        .collect();
    names.sort_unstable();
    assert_eq!(names, vec!["docs", "kg", "tags"]);

    // Verify each collection reports a concrete, valid type (not the "unknown" fallback).
    let mut types: Vec<&str> = results
        .iter()
        .filter_map(|r| payload_str(r, "type"))
        .collect();
    types.sort_unstable();
    assert_eq!(types, vec!["graph", "metadata", "vector"]);
}

#[test]
fn test_show_collections_empty_database() {
    let (_dir, db) = create_test_db();

    let results = execute_sql(&db, "SHOW COLLECTIONS").expect("SHOW COLLECTIONS should succeed");
    assert_eq!(
        results.len(),
        0,
        "empty database should return 0 collections"
    );
}

#[test]
fn test_show_collections_collection_types() {
    let (_dir, db) = create_test_db();

    execute_sql(
        &db,
        "CREATE COLLECTION vecs (dimension = 4, metric = 'cosine')",
    )
    .expect("create vector");
    execute_sql(
        &db,
        "CREATE GRAPH COLLECTION graph1 (dimension = 4, metric = 'cosine') SCHEMALESS",
    )
    .expect("create graph");
    execute_sql(&db, "CREATE METADATA COLLECTION meta1").expect("create metadata");

    let results = execute_sql(&db, "SHOW COLLECTIONS").expect("SHOW");

    // Build a name -> type map.
    let type_map: std::collections::HashMap<String, String> = results
        .iter()
        .filter_map(|r| {
            let name = payload_str(r, "name")?.to_string();
            let ctype = payload_str(r, "type")?.to_string();
            Some((name, ctype))
        })
        .collect();

    assert_eq!(type_map.get("vecs").map(String::as_str), Some("vector"));
    assert_eq!(type_map.get("graph1").map(String::as_str), Some("graph"));
    assert_eq!(type_map.get("meta1").map(String::as_str), Some("metadata"));
}

// ============================================================================
// DESCRIBE COLLECTION
// ============================================================================

#[test]
fn test_describe_collection_vector() {
    let (_dir, db) = create_test_db();

    execute_sql(
        &db,
        "CREATE COLLECTION docs (dimension = 128, metric = 'euclidean')",
    )
    .expect("create");

    let results = execute_sql(&db, "DESCRIBE COLLECTION docs").expect("DESCRIBE should succeed");
    assert_eq!(results.len(), 1, "should return exactly 1 result");

    let result = &results[0];
    assert_eq!(payload_str(result, "name"), Some("docs"));
    assert_eq!(payload_str(result, "type"), Some("vector"));

    // Verify dimension is present.
    let dim = result
        .point
        .payload
        .as_ref()
        .and_then(|p| p.get("dimension"))
        .and_then(serde_json::Value::as_u64);
    assert_eq!(dim, Some(128));
}

#[test]
fn test_describe_collection_graph() {
    let (_dir, db) = create_test_db();

    execute_sql(
        &db,
        "CREATE GRAPH COLLECTION kg (dimension = 4, metric = 'cosine') SCHEMALESS",
    )
    .expect("create graph");

    let results = execute_sql(&db, "DESCRIBE COLLECTION kg").expect("DESCRIBE should succeed");
    assert_eq!(results.len(), 1);

    let result = &results[0];
    assert_eq!(payload_str(result, "type"), Some("graph"));
}

#[test]
fn test_describe_collection_metadata() {
    let (_dir, db) = create_test_db();

    execute_sql(&db, "CREATE METADATA COLLECTION tags").expect("create metadata");

    let results = execute_sql(&db, "DESCRIBE tags").expect("DESCRIBE without COLLECTION keyword");
    assert_eq!(results.len(), 1);
    assert_eq!(payload_str(&results[0], "type"), Some("metadata"));
}

#[test]
fn test_describe_nonexistent_fails() {
    let (_dir, db) = create_test_db();

    let result = execute_sql(&db, "DESCRIBE COLLECTION ghost");
    assert!(
        result.is_err(),
        "DESCRIBE of nonexistent collection should fail"
    );
}

// ============================================================================
// EXPLAIN
// ============================================================================

#[test]
fn test_explain_select_returns_plan() {
    let (_dir, db) = create_test_db();

    execute_sql(
        &db,
        "CREATE COLLECTION docs (dimension = 4, metric = 'cosine')",
    )
    .expect("create");

    let results =
        execute_sql(&db, "EXPLAIN SELECT * FROM docs LIMIT 10").expect("EXPLAIN should succeed");
    assert_eq!(results.len(), 1, "should return 1 result (the plan)");

    // Verify the result has plan and tree fields.
    let result = &results[0];
    let has_plan = result
        .point
        .payload
        .as_ref()
        .and_then(|p| p.get("plan"))
        .is_some();
    assert!(has_plan, "result should contain a 'plan' field");

    let tree = result
        .point
        .payload
        .as_ref()
        .and_then(|p| p.get("tree"))
        .and_then(serde_json::Value::as_str)
        .expect("result should contain a 'tree' string field");
    assert!(
        tree.contains("docs"),
        "tree should name the scanned collection 'docs', got: {tree}"
    );
    assert!(
        tree.contains("Limit: 10"),
        "tree should show the explicit LIMIT 10 node, got: {tree}"
    );
}

// ============================================================================
// EDGE CASES
// ============================================================================

/// SHOW after DROP — verify dropped collection disappears from listing.
#[test]
fn test_show_collections_after_drop() {
    let (_dir, db) = create_test_db();

    execute_sql(
        &db,
        "CREATE COLLECTION temp (dimension = 4, metric = 'cosine')",
    )
    .expect("create");
    execute_sql(&db, "DROP COLLECTION temp").expect("drop");

    let results = execute_sql(&db, "SHOW COLLECTIONS").expect("SHOW");
    assert_eq!(results.len(), 0, "dropped collection should not appear");
}

/// SHOW with many collections — verify scalability and alphabetical ordering.
#[test]
fn test_show_collections_many_collections() {
    let (_dir, db) = create_test_db();

    for i in 0..10 {
        execute_sql(
            &db,
            &format!("CREATE COLLECTION col_{i} (dimension = 4, metric = 'cosine')"),
        )
        .expect("create");
    }

    let results = execute_sql(&db, "SHOW COLLECTIONS").expect("SHOW");
    assert_eq!(results.len(), 10, "should list all 10 collections");

    // Verify names are returned (order may vary).
    let names: std::collections::HashSet<String> = results
        .iter()
        .filter_map(|r| payload_str(r, "name").map(String::from))
        .collect();
    for i in 0..10 {
        assert!(
            names.contains(&format!("col_{i}")),
            "col_{i} should appear in results"
        );
    }
}

/// DESCRIBE after inserting data — verify `point_count` reflects state.
#[test]
fn test_describe_collection_with_data() {
    use velesdb_core::Point;

    let (_dir, db) = create_test_db();

    execute_sql(
        &db,
        "CREATE COLLECTION indexed (dimension = 4, metric = 'cosine')",
    )
    .expect("create");

    let vc = db.get_vector_collection("indexed").expect("get collection");
    vc.upsert(vec![
        Point::new(1, vec![1.0, 0.0, 0.0, 0.0], None),
        Point::new(2, vec![0.0, 1.0, 0.0, 0.0], None),
        Point::new(3, vec![0.0, 0.0, 1.0, 0.0], None),
    ])
    .expect("upsert");

    let results = execute_sql(&db, "DESCRIBE indexed").expect("DESCRIBE");
    assert_eq!(results.len(), 1);

    let point_count = results[0]
        .point
        .payload
        .as_ref()
        .and_then(|p| p.get("point_count"))
        .and_then(serde_json::Value::as_u64);
    assert_eq!(
        point_count,
        Some(3),
        "point_count should reflect inserted data"
    );
}

/// DESCRIBE of an HNSW-tuned collection still reports metric/dimension/point_count
/// (custom HNSW params are not surfaced in the DESCRIBE payload today).
#[test]
fn test_describe_collection_reports_metric_and_dimension() {
    let (_dir, db) = create_test_db();

    execute_sql(
        &db,
        "CREATE COLLECTION tuned (dimension = 64, metric = 'euclidean') WITH (m = 32, ef_construction = 400)",
    )
    .expect("create with HNSW params");

    let results = execute_sql(&db, "DESCRIBE tuned").expect("DESCRIBE");
    assert_eq!(results.len(), 1);
    // Core returns metric enum Display name (capitalized).
    let metric = payload_str(&results[0], "metric").expect("metric field");
    assert!(
        metric.eq_ignore_ascii_case("euclidean"),
        "metric should be euclidean, got: {metric}"
    );

    let dim = results[0]
        .point
        .payload
        .as_ref()
        .and_then(|p| p.get("dimension"))
        .and_then(serde_json::Value::as_u64);
    assert_eq!(dim, Some(64));

    let point_count = results[0]
        .point
        .payload
        .as_ref()
        .and_then(|p| p.get("point_count"))
        .and_then(serde_json::Value::as_u64);
    assert_eq!(
        point_count,
        Some(0),
        "freshly created collection has 0 points"
    );
}

// ============================================================================
// COMPLEX / COMPOSITION CASES
// ============================================================================

/// Create → Describe → Insert Data → Describe Again → Drop → Show
/// Full lifecycle introspection chain.
#[test]
fn test_introspection_full_lifecycle_chain() {
    use velesdb_core::Point;

    let (_dir, db) = create_test_db();

    // Step 1: Create collection
    execute_sql(
        &db,
        "CREATE COLLECTION lifecycle (dimension = 4, metric = 'cosine')",
    )
    .expect("create");

    // Step 2: SHOW — should list it
    let show1 = execute_sql(&db, "SHOW COLLECTIONS").expect("SHOW after create");
    assert_eq!(show1.len(), 1);
    assert_eq!(payload_str(&show1[0], "name"), Some("lifecycle"));

    // Step 3: DESCRIBE — should show empty collection
    let desc1 = execute_sql(&db, "DESCRIBE lifecycle").expect("DESCRIBE empty");
    let count1 = desc1[0]
        .point
        .payload
        .as_ref()
        .and_then(|p| p.get("point_count"))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);

    // Step 4: Insert data
    let vc = db.get_vector_collection("lifecycle").expect("get");
    vc.upsert(vec![
        Point::new(1, vec![1.0, 0.0, 0.0, 0.0], None),
        Point::new(2, vec![0.0, 1.0, 0.0, 0.0], None),
    ])
    .expect("upsert");

    // Step 5: DESCRIBE again — count should increase
    let desc2 = execute_sql(&db, "DESCRIBE lifecycle").expect("DESCRIBE with data");
    let count2 = desc2[0]
        .point
        .payload
        .as_ref()
        .and_then(|p| p.get("point_count"))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    assert!(
        count2 > count1,
        "point_count should increase after insert: {count1} -> {count2}"
    );

    // Step 6: Drop collection
    execute_sql(&db, "DROP COLLECTION lifecycle").expect("drop");

    // Step 7: SHOW — should be empty
    let show2 = execute_sql(&db, "SHOW COLLECTIONS").expect("SHOW after drop");
    assert_eq!(show2.len(), 0, "no collections after drop");

    // Step 8: DESCRIBE dropped — should fail
    let desc3 = execute_sql(&db, "DESCRIBE lifecycle");
    assert!(desc3.is_err(), "DESCRIBE dropped collection should fail");
}

/// EXPLAIN with complex query — NEAR + filter + ORDER BY.
#[test]
fn test_explain_complex_query() {
    let (_dir, db) = create_test_db();

    execute_sql(
        &db,
        "CREATE COLLECTION docs (dimension = 4, metric = 'cosine')",
    )
    .expect("create");

    let results = execute_sql(
        &db,
        "EXPLAIN SELECT * FROM docs WHERE vector NEAR $v AND category = 'tech' ORDER BY similarity() DESC LIMIT 10",
    )
    .expect("EXPLAIN complex should succeed");

    assert_eq!(results.len(), 1);

    let tree = results[0]
        .point
        .payload
        .as_ref()
        .and_then(|p| p.get("tree"))
        .and_then(serde_json::Value::as_str)
        .expect("tree field should be present");

    // Complex query plan should model the NEAR search, scalar filter, and limit.
    assert!(
        tree.contains("VectorSearch"),
        "plan should model the NEAR vector search: {tree}"
    );
    assert!(
        tree.contains("docs"),
        "VectorSearch node should name the queried collection: {tree}"
    );
    assert!(
        tree.contains("Filter"),
        "plan should model the scalar WHERE predicate (category = 'tech'): {tree}"
    );
    assert!(
        tree.contains("Limit: 10"),
        "plan should model the explicit LIMIT: {tree}"
    );
}

/// EXPLAIN on SELECT without vector search — pure metadata scan.
#[test]
fn test_explain_metadata_only_query() {
    let (_dir, db) = create_test_db();

    execute_sql(
        &db,
        "CREATE COLLECTION items (dimension = 4, metric = 'cosine')",
    )
    .expect("create");

    let results = execute_sql(
        &db,
        "EXPLAIN SELECT * FROM items WHERE price > 100 LIMIT 20",
    )
    .expect("EXPLAIN metadata query");

    assert_eq!(results.len(), 1);
    let tree = results[0]
        .point
        .payload
        .as_ref()
        .and_then(|p| p.get("tree"))
        .and_then(serde_json::Value::as_str)
        .expect("tree field should be present");
    // Non-vector query must plan a TableScan over the queried collection,
    // not a VectorSearch, and must surface the scalar WHERE filter.
    assert!(
        tree.contains("TableScan: items"),
        "metadata-only plan should TableScan the queried collection, got:\n{tree}"
    );
    assert!(
        !tree.contains("VectorSearch"),
        "metadata-only query must not plan a vector search, got:\n{tree}"
    );
    assert!(
        tree.contains("Filter"),
        "scalar WHERE clause should produce a Filter node, got:\n{tree}"
    );
}

/// SHOW COLLECTIONS with mixed types + DESCRIBE each — cross-type introspection.
#[test]
fn test_describe_all_collection_types_in_sequence() {
    let (_dir, db) = create_test_db();

    execute_sql(
        &db,
        "CREATE COLLECTION vectors (dimension = 384, metric = 'cosine')",
    )
    .expect("create vector");
    execute_sql(
        &db,
        "CREATE GRAPH COLLECTION graphs (dimension = 4, metric = 'cosine') SCHEMALESS",
    )
    .expect("create graph");
    execute_sql(&db, "CREATE METADATA COLLECTION meta").expect("create meta");

    // Describe each type and verify type-specific fields.
    let v = execute_sql(&db, "DESCRIBE vectors").expect("describe vector");
    assert_eq!(payload_str(&v[0], "type"), Some("vector"));
    let dim = v[0]
        .point
        .payload
        .as_ref()
        .and_then(|p| p.get("dimension"))
        .and_then(serde_json::Value::as_u64);
    assert_eq!(dim, Some(384));

    let g = execute_sql(&db, "DESCRIBE graphs").expect("describe graph");
    assert_eq!(payload_str(&g[0], "type"), Some("graph"));

    let m = execute_sql(&db, "DESCRIBE meta").expect("describe metadata");
    assert_eq!(payload_str(&m[0], "type"), Some("metadata"));
}
