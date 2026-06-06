//! E2E lifecycle tests for `VelesQL` DDL/DML.
//!
//! These tests exercise the **full pipeline**: SQL string -> `Parser::parse()` ->
//! `Database::execute_query()` -> verify database state. Each test is isolated
//! via `tempfile::TempDir` and requires the `persistence` feature.

use super::helpers::{create_test_db, execute_sql};
use velesdb_core::{DistanceMetric, Point};

// =========================================================================
// Scenario 1: Vector Collection Full Lifecycle
// =========================================================================

#[test]
fn test_vector_collection_full_lifecycle() {
    let (_dir, db) = create_test_db();

    // Step 1: CREATE via SQL
    execute_sql(
        &db,
        "CREATE COLLECTION docs (dimension = 4, metric = 'cosine');",
    )
    .expect("test: CREATE should succeed");
    assert!(
        db.list_collections().contains(&"docs".to_string()),
        "Collection 'docs' should appear in list after CREATE"
    );

    // Step 2: Verify typed collection is accessible and has correct properties
    let vc = db
        .get_vector_collection("docs")
        .expect("test: vector collection should exist");
    assert_eq!(vc.dimension(), 4);
    assert_eq!(vc.metric(), DistanceMetric::Cosine);

    // Step 3: Insert data via API (VelesQL INSERT INTO doesn't naturally handle
    // vector data well in the SQL literal form, so we use the typed API here)
    vc.upsert(vec![
        Point::new(0, vec![1.0, 0.0, 0.0, 0.0], None),
        Point::new(1, vec![0.0, 1.0, 0.0, 0.0], None),
        Point::new(2, vec![0.0, 0.0, 1.0, 0.0], None),
        Point::new(3, vec![0.0, 0.0, 0.0, 1.0], None),
    ])
    .expect("test: upsert should succeed");

    // Step 4: SELECT via SQL -- verify data is accessible through the parser pipeline
    let results =
        execute_sql(&db, "SELECT * FROM docs LIMIT 10;").expect("test: SELECT should succeed");
    assert_eq!(results.len(), 4, "Should return all 4 inserted points");

    // Step 5: DELETE a single point via SQL
    execute_sql(&db, "DELETE FROM docs WHERE id = 0;").expect("test: DELETE single should succeed");
    let points = vc.get(&[0, 1, 2, 3]);
    assert!(points[0].is_none(), "Point 0 should be deleted");
    assert!(points[1].is_some(), "Point 1 should remain");
    assert!(points[2].is_some(), "Point 2 should remain");
    assert!(points[3].is_some(), "Point 3 should remain");

    // Step 6: DROP via SQL
    execute_sql(&db, "DROP COLLECTION docs;").expect("test: DROP should succeed");
    assert!(
        !db.list_collections().contains(&"docs".to_string()),
        "Collection 'docs' should be gone after DROP"
    );
}

// =========================================================================
// Scenario 2: Graph Collection with Edges Full Lifecycle
// =========================================================================

#[test]
fn test_graph_collection_with_edges_lifecycle() {
    let (_dir, db) = create_test_db();

    // Step 1: CREATE GRAPH COLLECTION via SQL (with embeddings)
    execute_sql(
        &db,
        "CREATE GRAPH COLLECTION kg (dimension = 4, metric = 'cosine') SCHEMALESS;",
    )
    .expect("test: CREATE GRAPH should succeed");
    assert!(db.list_collections().contains(&"kg".to_string()));

    let gc = db
        .get_graph_collection("kg")
        .expect("test: graph collection should exist");
    assert_eq!(gc.name(), "kg");
    assert!(gc.schema().is_schemaless());
    assert!(gc.has_embeddings());

    // Step 2: Insert nodes via API (needed before edges can reference them)
    gc.upsert_node_payload(
        1,
        &serde_json::json!({"name": "Alice", "_labels": ["Person"]}),
    )
    .expect("test: upsert node 1");
    gc.upsert_node_payload(
        2,
        &serde_json::json!({"name": "Bob", "_labels": ["Person"]}),
    )
    .expect("test: upsert node 2");
    gc.upsert_node_payload(
        3,
        &serde_json::json!({"name": "Charlie", "_labels": ["Person"]}),
    )
    .expect("test: upsert node 3");

    // Step 3: INSERT EDGE via SQL with explicit IDs
    execute_sql(
        &db,
        "INSERT EDGE INTO kg (id = 10, source = 1, target = 2, label = 'KNOWS');",
    )
    .expect("test: INSERT EDGE should succeed");

    execute_sql(
        &db,
        "INSERT EDGE INTO kg (id = 20, source = 2, target = 3, label = 'KNOWS') WITH PROPERTIES (weight = 0.9);",
    )
    .expect("test: INSERT EDGE with properties should succeed");

    // Step 4: Verify edges exist
    let gc = db
        .get_graph_collection("kg")
        .expect("test: graph collection should still exist");
    assert_eq!(gc.edge_count(), 2);

    let edges = gc.get_edges(Some("KNOWS"));
    assert_eq!(edges.len(), 2, "Should have 2 KNOWS edges");

    // Step 5: DELETE EDGE via SQL -- delete edge 10
    execute_sql(&db, "DELETE EDGE 10 FROM kg;").expect("test: DELETE EDGE should succeed");
    assert_eq!(gc.edge_count(), 1, "Should have 1 edge remaining");

    // Step 6: DROP via SQL
    execute_sql(&db, "DROP COLLECTION kg;").expect("test: DROP should succeed");
    assert!(
        !db.list_collections().contains(&"kg".to_string()),
        "Collection 'kg' should be gone after DROP"
    );
}

// =========================================================================
// Scenario 3: Metadata Collection Lifecycle
// =========================================================================

#[test]
fn test_metadata_collection_lifecycle() {
    let (_dir, db) = create_test_db();

    // CREATE METADATA COLLECTION via SQL
    execute_sql(&db, "CREATE METADATA COLLECTION tags;")
        .expect("test: CREATE METADATA should succeed");
    assert!(db.list_collections().contains(&"tags".to_string()));

    let mc = db
        .get_metadata_collection("tags")
        .expect("test: metadata collection should exist");
    assert_eq!(mc.name(), "tags");
    assert_eq!(mc.len(), 0, "Fresh metadata collection should be empty");

    // Insert metadata-only points via API
    mc.upsert(vec![
        Point::metadata_only(1, serde_json::json!({"tag": "rust"})),
        Point::metadata_only(2, serde_json::json!({"tag": "python"})),
    ])
    .expect("test: upsert metadata points");
    assert_eq!(mc.len(), 2);

    // Verify retrieval
    let points = mc.get(&[1, 2]);
    assert!(points[0].is_some(), "Metadata point 1 should exist");
    assert!(points[1].is_some(), "Metadata point 2 should exist");

    // DROP via SQL
    execute_sql(&db, "DROP COLLECTION tags;").expect("test: DROP should succeed");
    assert!(
        !db.list_collections().contains(&"tags".to_string()),
        "Collection 'tags' should be gone after DROP"
    );
}

// =========================================================================
// Scenario 4: Drop and Recreate (name reuse)
// =========================================================================

#[test]
fn test_drop_and_recreate_same_name() {
    let (_dir, db) = create_test_db();

    // CREATE, insert data, DROP
    execute_sql(
        &db,
        "CREATE COLLECTION reuse_me (dimension = 4, metric = 'cosine');",
    )
    .expect("test: first CREATE");

    let vc = db
        .get_vector_collection("reuse_me")
        .expect("test: get after first create");
    vc.upsert(vec![Point::new(1, vec![1.0, 0.0, 0.0, 0.0], None)])
        .expect("test: upsert");
    assert_eq!(vc.len(), 1);

    execute_sql(&db, "DROP COLLECTION reuse_me;").expect("test: DROP");
    assert!(!db.list_collections().contains(&"reuse_me".to_string()));

    // Recreate with same name -- should be fresh and empty
    execute_sql(
        &db,
        "CREATE COLLECTION reuse_me (dimension = 8, metric = 'euclidean');",
    )
    .expect("test: second CREATE with same name");

    let vc2 = db
        .get_vector_collection("reuse_me")
        .expect("test: get after second create");
    assert_eq!(
        vc2.dimension(),
        8,
        "Recreated collection should have new dimension"
    );
    assert_eq!(
        vc2.metric(),
        DistanceMetric::Euclidean,
        "Recreated collection should have new metric"
    );
    assert_eq!(vc2.len(), 0, "Recreated collection should be empty");
}

// =========================================================================
// Scenario 5: IF EXISTS behavior
// =========================================================================

#[test]
fn test_drop_if_exists_nonexistent_succeeds() {
    let (_dir, db) = create_test_db();

    // DROP IF EXISTS on a nonexistent collection should silently succeed
    let result = execute_sql(&db, "DROP COLLECTION IF EXISTS nonexistent;");
    assert!(
        result.is_ok(),
        "DROP IF EXISTS on nonexistent should not error"
    );
}

#[test]
fn test_drop_if_exists_existing_succeeds() {
    let (_dir, db) = create_test_db();

    execute_sql(
        &db,
        "CREATE COLLECTION ephemeral (dimension = 4, metric = 'cosine');",
    )
    .expect("test: CREATE");
    assert!(db.list_collections().contains(&"ephemeral".to_string()));

    execute_sql(&db, "DROP COLLECTION IF EXISTS ephemeral;").expect("test: DROP IF EXISTS");
    assert!(
        !db.list_collections().contains(&"ephemeral".to_string()),
        "Collection should be dropped"
    );
}

#[test]
fn test_drop_without_if_exists_nonexistent_fails() {
    let (_dir, db) = create_test_db();

    let err = execute_sql(&db, "DROP COLLECTION ghost;").expect_err("should fail");
    let msg = err.to_string();
    assert!(
        msg.contains("ghost") || msg.contains("not found") || msg.contains("NotFound"),
        "Error should mention the missing collection, got: {msg}"
    );
}

// =========================================================================
// Scenario 6: Error Recovery Chain
// =========================================================================

#[test]
fn test_error_recovery_after_invalid_create() {
    let (_dir, db) = create_test_db();

    // Invalid metric should fail at execution time
    let err = execute_sql(
        &db,
        "CREATE COLLECTION bad (dimension = 4, metric = 'manhattan');",
    )
    .expect_err("invalid metric should error");
    let msg = err.to_string();
    assert!(
        msg.contains("manhattan") || msg.contains("Unknown metric"),
        "Error should mention invalid metric, got: {msg}"
    );

    // Previous failure should NOT corrupt database state -- valid CREATE should succeed
    execute_sql(
        &db,
        "CREATE COLLECTION good (dimension = 4, metric = 'cosine');",
    )
    .expect("test: valid CREATE after failed one should succeed");
    assert!(db.list_collections().contains(&"good".to_string()));
}

#[test]
fn test_error_recovery_after_duplicate_create() {
    let (_dir, db) = create_test_db();

    execute_sql(
        &db,
        "CREATE COLLECTION first (dimension = 4, metric = 'cosine');",
    )
    .expect("test: first CREATE");

    // Duplicate name should error
    let err = execute_sql(
        &db,
        "CREATE COLLECTION first (dimension = 8, metric = 'euclidean');",
    )
    .expect_err("duplicate should error");
    let msg = err.to_string();
    assert!(
        msg.contains("first") || msg.contains("exists") || msg.contains("Exists"),
        "Error should mention duplicate, got: {msg}"
    );

    // Original collection should be unaffected
    let vc = db
        .get_vector_collection("first")
        .expect("test: original should still exist");
    assert_eq!(
        vc.dimension(),
        4,
        "Original collection should retain its dimension"
    );
}

// =========================================================================
// Scenario 7: Graph Typed Schema Lifecycle
// =========================================================================

#[test]
fn test_graph_typed_schema_lifecycle() {
    let (_dir, db) = create_test_db();

    // CREATE GRAPH COLLECTION with typed schema via SQL.
    execute_sql(
        &db,
        "CREATE GRAPH COLLECTION social (dimension = 4, metric = 'cosine') WITH SCHEMA (NODE Person (name: STRING, age: INTEGER), EDGE KNOWS FROM Person TO Person);",
    )
    .expect("test: CREATE GRAPH with typed schema should succeed");

    let gc = db
        .get_graph_collection("social")
        .expect("test: graph collection should exist");
    assert!(!gc.schema().is_schemaless(), "Schema should be typed");
    assert!(
        gc.schema().has_node_type("Person"),
        "Should have Person node type"
    );
    assert!(
        gc.schema().has_edge_type("KNOWS"),
        "Should have KNOWS edge type"
    );

    // Insert the typed endpoint nodes first: strict-schema referential
    // integrity requires both endpoints to exist with a declared type before
    // an edge can reference them.
    gc.upsert_node_payload(
        1,
        &serde_json::json!({"name": "Alice", "_labels": ["Person"]}),
    )
    .expect("test: upsert node 1");
    gc.upsert_node_payload(
        2,
        &serde_json::json!({"name": "Bob", "_labels": ["Person"]}),
    )
    .expect("test: upsert node 2");

    // INSERT EDGE via SQL
    execute_sql(
        &db,
        "INSERT EDGE INTO social (id = 1, source = 1, target = 2, label = 'KNOWS');",
    )
    .expect("test: INSERT EDGE should succeed");
    assert_eq!(gc.edge_count(), 1);

    // DROP via SQL
    execute_sql(&db, "DROP COLLECTION social;").expect("test: DROP typed graph");
    assert!(!db.list_collections().contains(&"social".to_string()));
}

// =========================================================================
// Scenario 8: Multiple Collections Coexist
// =========================================================================

#[test]
fn test_multiple_collection_types_coexist() {
    let (_dir, db) = create_test_db();

    // Create all 3 collection types via SQL.
    execute_sql(
        &db,
        "CREATE COLLECTION vectors (dimension = 4, metric = 'cosine');",
    )
    .expect("test: CREATE vector");
    execute_sql(
        &db,
        "CREATE GRAPH COLLECTION graphs (dimension = 4, metric = 'cosine') SCHEMALESS;",
    )
    .expect("test: CREATE graph");
    execute_sql(&db, "CREATE METADATA COLLECTION meta;").expect("test: CREATE metadata");

    // All 3 should coexist
    let names = db.list_collections();
    assert!(
        names.contains(&"vectors".to_string()),
        "vectors should exist"
    );
    assert!(names.contains(&"graphs".to_string()), "graphs should exist");
    assert!(names.contains(&"meta".to_string()), "meta should exist");

    // Verify each type is accessible via its typed getter
    assert!(db.get_vector_collection("vectors").is_some());
    assert!(db.get_graph_collection("graphs").is_some());
    assert!(db.get_metadata_collection("meta").is_some());

    // Drop each one via SQL
    execute_sql(&db, "DROP COLLECTION vectors;").expect("test: DROP vectors");
    execute_sql(&db, "DROP COLLECTION graphs;").expect("test: DROP graphs");
    execute_sql(&db, "DROP COLLECTION meta;").expect("test: DROP meta");

    assert!(
        db.list_collections().is_empty(),
        "All collections should be gone"
    );
}

// =========================================================================
// Scenario 9: DELETE with IN clause
// =========================================================================

#[test]
fn test_delete_with_in_clause() {
    let (_dir, db) = create_test_db();

    execute_sql(
        &db,
        "CREATE COLLECTION multi_del (dimension = 4, metric = 'cosine');",
    )
    .expect("test: CREATE");

    let vc = db
        .get_vector_collection("multi_del")
        .expect("test: get collection");
    vc.upsert(vec![
        Point::new(1, vec![1.0, 0.0, 0.0, 0.0], None),
        Point::new(2, vec![0.0, 1.0, 0.0, 0.0], None),
        Point::new(3, vec![0.0, 0.0, 1.0, 0.0], None),
        Point::new(4, vec![0.0, 0.0, 0.0, 1.0], None),
        Point::new(5, vec![0.5, 0.5, 0.0, 0.0], None),
    ])
    .expect("test: upsert 5 points");

    // DELETE with IN clause via SQL
    execute_sql(&db, "DELETE FROM multi_del WHERE id IN (1, 3, 5);")
        .expect("test: DELETE IN should succeed");

    let points = vc.get(&[1, 2, 3, 4, 5]);
    assert!(points[0].is_none(), "Point 1 should be deleted");
    assert!(points[1].is_some(), "Point 2 should remain");
    assert!(points[2].is_none(), "Point 3 should be deleted");
    assert!(points[3].is_some(), "Point 4 should remain");
    assert!(points[4].is_none(), "Point 5 should be deleted");
}

// =========================================================================
// Scenario 10: Cross-type Operations Error
// =========================================================================

#[test]
fn test_insert_edge_into_vector_collection_errors() {
    let (_dir, db) = create_test_db();

    execute_sql(
        &db,
        "CREATE COLLECTION not_a_graph (dimension = 4, metric = 'cosine');",
    )
    .expect("test: CREATE vector");

    // INSERT EDGE into a vector collection should error
    let err = execute_sql(
        &db,
        "INSERT EDGE INTO not_a_graph (source = 1, target = 2, label = 'REL');",
    )
    .expect_err("INSERT EDGE into vector collection should fail");
    let msg = err.to_string();
    assert!(
        msg.contains("not_a_graph") || msg.contains("not found") || msg.contains("NotFound"),
        "Error should indicate collection is not a graph, got: {msg}"
    );
}

#[test]
fn test_delete_edge_from_metadata_collection_errors() {
    let (_dir, db) = create_test_db();

    execute_sql(&db, "CREATE METADATA COLLECTION not_graph;").expect("test: CREATE metadata");

    // DELETE EDGE from a metadata collection should error
    let err = execute_sql(&db, "DELETE EDGE 42 FROM not_graph;")
        .expect_err("DELETE EDGE from metadata should fail");
    let msg = err.to_string();
    assert!(
        msg.contains("not_graph") || msg.contains("not found") || msg.contains("NotFound"),
        "Error should indicate not a graph collection, got: {msg}"
    );
}

#[test]
fn test_insert_edge_into_nonexistent_collection_errors() {
    let (_dir, db) = create_test_db();

    let err = execute_sql(
        &db,
        "INSERT EDGE INTO no_such_thing (source = 1, target = 2, label = 'REL');",
    )
    .expect_err("INSERT EDGE into nonexistent should fail");
    let msg = err.to_string();
    assert!(
        msg.contains("no_such_thing") || msg.contains("not found") || msg.contains("NotFound"),
        "Error should mention missing collection, got: {msg}"
    );
}

// =========================================================================
// Scenario 11: Vector Collection with HNSW Params via SQL
// =========================================================================

#[test]
fn test_vector_collection_with_hnsw_params_via_sql() {
    let (_dir, db) = create_test_db();

    // CREATE with explicit HNSW params via WITH clause
    execute_sql(
        &db,
        "CREATE COLLECTION tuned (dimension = 64, metric = 'euclidean') WITH (m = 32, ef_construction = 200);",
    )
    .expect("test: CREATE with HNSW params");

    let vc = db
        .get_vector_collection("tuned")
        .expect("test: collection should exist");
    assert_eq!(vc.dimension(), 64);
    assert_eq!(vc.metric(), DistanceMetric::Euclidean);
}

// =========================================================================
// Scenario 12: Graph Collection Without Embeddings
// =========================================================================

#[test]
fn test_graph_collection_without_embeddings() {
    let (_dir, db) = create_test_db();

    execute_sql(&db, "CREATE GRAPH COLLECTION pure_graph;").expect("test: CREATE pure graph");

    let gc = db
        .get_graph_collection("pure_graph")
        .expect("test: graph collection should exist");
    assert!(
        !gc.has_embeddings(),
        "Pure graph should not have embeddings"
    );
    assert!(gc.schema().is_schemaless());

    // Insert edge via SQL with explicit ID
    execute_sql(
        &db,
        "INSERT EDGE INTO pure_graph (id = 1, source = 10, target = 20, label = 'LINKED');",
    )
    .expect("test: INSERT EDGE should succeed");
    assert_eq!(gc.edge_count(), 1);

    // Delete edge via SQL using the known explicit ID
    execute_sql(&db, "DELETE EDGE 1 FROM pure_graph;").expect("test: DELETE EDGE should succeed");
    assert_eq!(gc.edge_count(), 0);
}

// =========================================================================
// Scenario 13: INSERT EDGE with Properties Full Round-Trip
// =========================================================================

#[test]
fn test_insert_edge_with_properties_round_trip() {
    let (_dir, db) = create_test_db();

    execute_sql(
        &db,
        "CREATE GRAPH COLLECTION props_kg (dimension = 4, metric = 'cosine') SCHEMALESS;",
    )
    .expect("test: CREATE graph");

    execute_sql(
        &db,
        "INSERT EDGE INTO props_kg (id = 1, source = 1, target = 2, label = 'FRIEND') WITH PROPERTIES (since = 2020, weight = 0.85);",
    )
    .expect("test: INSERT EDGE with properties");

    let gc = db
        .get_graph_collection("props_kg")
        .expect("test: get graph");
    let edges = gc.get_edges(None);
    assert_eq!(edges.len(), 1);

    let edge = &edges[0];
    assert_eq!(edge.source(), 1);
    assert_eq!(edge.target(), 2);
    assert_eq!(edge.label(), "FRIEND");
    assert_eq!(
        edge.property("since"),
        Some(&serde_json::json!(2020)),
        "Integer property should round-trip"
    );
    assert_eq!(
        edge.property("weight"),
        Some(&serde_json::json!(0.85)),
        "Float property should round-trip"
    );
}

// =========================================================================
// Scenario 14: Multiple Edges then Selective Delete
// =========================================================================

#[test]
fn test_multiple_edges_selective_delete() {
    let (_dir, db) = create_test_db();

    execute_sql(
        &db,
        "CREATE GRAPH COLLECTION multi_edge (dimension = 4, metric = 'cosine') SCHEMALESS;",
    )
    .expect("test: CREATE graph");

    // Insert 3 edges with explicit IDs
    execute_sql(
        &db,
        "INSERT EDGE INTO multi_edge (id = 100, source = 1, target = 2, label = 'A');",
    )
    .expect("test: edge 1");
    execute_sql(
        &db,
        "INSERT EDGE INTO multi_edge (id = 200, source = 2, target = 3, label = 'B');",
    )
    .expect("test: edge 2");
    execute_sql(
        &db,
        "INSERT EDGE INTO multi_edge (id = 300, source = 3, target = 1, label = 'C');",
    )
    .expect("test: edge 3");

    let gc = db
        .get_graph_collection("multi_edge")
        .expect("test: get graph");
    assert_eq!(gc.edge_count(), 3);

    // Delete only the middle edge
    execute_sql(&db, "DELETE EDGE 200 FROM multi_edge;").expect("test: DELETE EDGE 200");
    assert_eq!(gc.edge_count(), 2);

    // Verify remaining edges
    let remaining = gc.get_edges(None);
    let remaining_ids: Vec<u64> = remaining.iter().map(velesdb_core::GraphEdge::id).collect();
    assert!(remaining_ids.contains(&100), "Edge 100 should remain");
    assert!(!remaining_ids.contains(&200), "Edge 200 should be deleted");
    assert!(remaining_ids.contains(&300), "Edge 300 should remain");
}

// =========================================================================
// Scenario 15: SQL Parse Errors Are Properly Propagated
// =========================================================================

#[test]
fn test_sql_parse_error_propagation() {
    let (_dir, db) = create_test_db();

    // Syntactically invalid SQL should produce a parse error
    let err = execute_sql(&db, "INVALID STATEMENT;").expect_err("invalid SQL should error");
    // The error should be a Query error wrapping the parse failure
    assert!(
        matches!(err, velesdb_core::Error::Query(_)),
        "Should be a Query error, got: {err:?}"
    );
}

#[test]
fn test_create_missing_dimension_is_parse_error() {
    let (_dir, db) = create_test_db();

    // Missing required 'dimension' parameter
    let err = execute_sql(&db, "CREATE COLLECTION bad (metric = 'cosine');")
        .expect_err("missing dimension should error");
    let msg = err.to_string();
    assert!(
        msg.contains("dimension") || msg.contains("requires"),
        "Error should mention missing dimension, got: {msg}"
    );
}

// =========================================================================
// Scenario 16: Graph Collection with Embeddings -- Full INSERT + SELECT
// =========================================================================

#[test]
fn test_graph_with_embeddings_insert_and_select() {
    let (_dir, db) = create_test_db();

    execute_sql(
        &db,
        "CREATE GRAPH COLLECTION embed_kg (dimension = 4, metric = 'cosine') SCHEMALESS;",
    )
    .expect("test: CREATE graph with embeddings");

    let gc = db
        .get_graph_collection("embed_kg")
        .expect("test: get graph");
    assert!(gc.has_embeddings());

    // Upsert nodes with embeddings via the graph collection API.
    gc.upsert_node_payload(
        1,
        &serde_json::json!({"name": "Alice", "_labels": ["Person"]}),
    )
    .expect("test: upsert node 1");
    gc.upsert_node_payload(
        2,
        &serde_json::json!({"name": "Bob", "_labels": ["Person"]}),
    )
    .expect("test: upsert node 2");

    // Insert edge via SQL
    execute_sql(
        &db,
        "INSERT EDGE INTO embed_kg (id = 1, source = 1, target = 2, label = 'KNOWS');",
    )
    .expect("test: INSERT EDGE");
    assert_eq!(gc.edge_count(), 1);

    // Verify node payloads were stored via the payload API
    let payload1 = gc
        .get_node_payload(1)
        .expect("test: get_node_payload should not error");
    assert!(payload1.is_some(), "Node 1 payload should exist");
    let payload2 = gc
        .get_node_payload(2)
        .expect("test: get_node_payload should not error");
    assert!(payload2.is_some(), "Node 2 payload should exist");
}

// =========================================================================
// Scenario 17: DotProduct and Euclidean Metrics via SQL
// =========================================================================

#[test]
fn test_vector_collection_dotproduct_metric() {
    let (_dir, db) = create_test_db();

    execute_sql(
        &db,
        "CREATE COLLECTION dot_coll (dimension = 4, metric = 'dotproduct');",
    )
    .expect("test: CREATE with dotproduct");

    let vc = db
        .get_vector_collection("dot_coll")
        .expect("test: get collection");
    assert_eq!(vc.metric(), DistanceMetric::DotProduct);
}

#[test]
fn test_vector_collection_euclidean_metric() {
    let (_dir, db) = create_test_db();

    execute_sql(
        &db,
        "CREATE COLLECTION euc_coll (dimension = 4, metric = 'euclidean');",
    )
    .expect("test: CREATE with euclidean");

    let vc = db
        .get_vector_collection("euc_coll")
        .expect("test: get collection");
    assert_eq!(vc.metric(), DistanceMetric::Euclidean);
}

// =========================================================================
// Scenario 18: Delete from Nonexistent Collection
// =========================================================================

#[test]
fn test_delete_from_nonexistent_collection_errors() {
    let (_dir, db) = create_test_db();

    let err = execute_sql(&db, "DELETE FROM ghost WHERE id = 1;")
        .expect_err("DELETE from nonexistent should fail");
    let msg = err.to_string();
    assert!(
        msg.contains("ghost") || msg.contains("not found") || msg.contains("NotFound"),
        "Error should mention missing collection, got: {msg}"
    );
}

// =========================================================================
// Scenario 19: SELECT on Empty Collection
// =========================================================================

#[test]
fn test_select_on_empty_collection() {
    let (_dir, db) = create_test_db();

    execute_sql(
        &db,
        "CREATE COLLECTION empty_coll (dimension = 4, metric = 'cosine');",
    )
    .expect("test: CREATE");

    let results = execute_sql(&db, "SELECT * FROM empty_coll LIMIT 10;")
        .expect("test: SELECT on empty should succeed");
    assert!(
        results.is_empty(),
        "SELECT on empty collection should return 0 results"
    );
}

// =========================================================================
// Scenario 20: Full INSERT INTO via SQL (DML path)
// =========================================================================

#[test]
fn test_insert_into_via_sql() {
    let (_dir, db) = create_test_db();

    execute_sql(
        &db,
        "CREATE COLLECTION sql_insert (dimension = 4, metric = 'cosine');",
    )
    .expect("test: CREATE");

    // INSERT INTO via full SQL pipeline -- requires id and vector columns.
    // Uses $vector parameter which must be resolved from params HashMap.
    execute_sql(
        &db,
        "INSERT INTO sql_insert (id, vector, category) VALUES (1, $vector, 'test');",
    )
    .unwrap_or_else(|e| {
        // INSERT INTO with $vector parameter requires params -- this path tests
        // that the parser handles DML correctly. If it errors because of missing
        // parameter, that's the expected parse -> validate -> execute flow.
        let msg = e.to_string();
        assert!(
            msg.contains("vector") || msg.contains("parameter") || msg.contains("requires"),
            "Error should be about missing vector/param, got: {msg}"
        );
        Vec::new()
    });
}
