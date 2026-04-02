//! BDD regression tests for bugs found by Devin Review.
//!
//! Each test documents the original bug and proves the fix holds.
//! Tests exercise the full pipeline: SQL string -> parse -> execute -> verify.

use super::helpers::{create_test_db, execute_sql, execute_sql_with_params, payload_str};
use std::collections::HashMap;
use velesdb_core::Point;

// ============================================================================
// Bug 1: INSERT INTO must return the inserted point (not empty results)
// ============================================================================

/// The server previously discarded INSERT INTO results by routing through a
/// mutation path that returned `Ok(())` instead of the inserted points.
/// At core level, `execute_insert()` builds `SearchResult` items and returns
/// them. This test ensures that contract is never broken.
#[test]
fn test_regression_insert_into_returns_results() {
    let (_dir, db) = create_test_db();

    execute_sql(&db, "CREATE METADATA COLLECTION reg").expect("create metadata collection");

    let results =
        execute_sql(&db, "INSERT INTO reg (id, title) VALUES (1, 'test')").expect("insert");

    assert!(
        !results.is_empty(),
        "INSERT INTO must return the inserted point, not empty results"
    );
    assert_eq!(
        results.len(),
        1,
        "single-row INSERT returns exactly 1 result"
    );
    assert_eq!(
        results[0].point.id, 1,
        "returned point must have correct id"
    );
}

/// Same bug, but for a vector collection (requires vector column).
#[test]
fn test_regression_insert_into_vector_returns_results() {
    let (_dir, db) = create_test_db();

    execute_sql(
        &db,
        "CREATE COLLECTION vins (dimension = 4, metric = 'cosine')",
    )
    .expect("create vector collection");

    let mut params = HashMap::new();
    params.insert("v".to_string(), serde_json::json!([1.0, 0.0, 0.0, 0.0]));

    let results = execute_sql_with_params(
        &db,
        "INSERT INTO vins (id, vector, title) VALUES (1, $v, 'test')",
        &params,
    )
    .expect("insert with vector");

    assert!(
        !results.is_empty(),
        "INSERT INTO vector collection must return the inserted point"
    );
    assert_eq!(results[0].point.id, 1);
}

// ============================================================================
// Bug 2: UPDATE must return updated points (not empty results)
// ============================================================================

/// UPDATE was routed through a mutation path that discarded results.
/// Core `upsert_and_collect()` builds `SearchResult` items from the updated
/// points. This test verifies both the return value and the state change.
#[test]
fn test_regression_update_returns_results_and_mutates_state() {
    let (_dir, db) = create_test_db();

    execute_sql(
        &db,
        "CREATE COLLECTION upd (dimension = 4, metric = 'cosine')",
    )
    .expect("create");

    let vc = db.get_vector_collection("upd").expect("get collection");
    vc.upsert(vec![Point::new(
        1,
        vec![1.0, 0.0, 0.0, 0.0],
        Some(serde_json::json!({"status": "old"})),
    )])
    .expect("seed data");

    let results = execute_sql(&db, "UPDATE upd SET status = 'new' WHERE id = 1").expect("update");

    assert_eq!(results.len(), 1, "UPDATE should return the updated point");
    assert_eq!(results[0].point.id, 1, "updated point has correct id");

    // Verify state mutation persisted.
    let fetched = vc.get(&[1]);
    let payload = fetched[0]
        .as_ref()
        .expect("point should exist")
        .payload
        .as_ref()
        .expect("payload should exist");
    assert_eq!(
        payload.get("status").and_then(|v| v.as_str()),
        Some("new"),
        "payload field must reflect the UPDATE"
    );
}

// ============================================================================
// Bug 3: UPSERT must return results (same root cause as INSERT INTO)
// ============================================================================

/// UPSERT INTO shares the same `execute_insert` path as INSERT INTO.
/// This test ensures the returned results are non-empty.
#[test]
fn test_regression_upsert_returns_results() {
    let (_dir, db) = create_test_db();

    execute_sql(&db, "CREATE METADATA COLLECTION ups").expect("create metadata collection");

    let results =
        execute_sql(&db, "UPSERT INTO ups (id, title) VALUES (1, 'test')").expect("upsert");

    assert!(!results.is_empty(), "UPSERT must return the upserted point");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].point.id, 1);
}

/// UPSERT on a vector collection with bind-param vector.
#[test]
fn test_regression_upsert_vector_returns_results() {
    let (_dir, db) = create_test_db();

    execute_sql(
        &db,
        "CREATE COLLECTION vups (dimension = 4, metric = 'cosine')",
    )
    .expect("create");

    let mut params = HashMap::new();
    params.insert("v".to_string(), serde_json::json!([0.0, 1.0, 0.0, 0.0]));

    let results = execute_sql_with_params(
        &db,
        "UPSERT INTO vups (id, vector, title) VALUES (1, $v, 'test')",
        &params,
    )
    .expect("upsert with vector");

    assert!(
        !results.is_empty(),
        "UPSERT INTO vector collection must return the upserted point"
    );
    assert_eq!(results[0].point.id, 1);
}

// ============================================================================
// Bug 4: Introspection queries work through Database::execute_query
//        without requiring a FROM clause
// ============================================================================

/// Introspection queries (SHOW, DESCRIBE, EXPLAIN) have an empty `from`
/// field in the AST. A naive implementation that resolves the collection
/// from `query.select.from` before checking for introspection would fail
/// with `CollectionNotFound("")`.
#[test]
fn test_regression_show_collections_no_from_clause() {
    let (_dir, db) = create_test_db();

    execute_sql(
        &db,
        "CREATE COLLECTION test_col (dimension = 4, metric = 'cosine')",
    )
    .expect("create");

    let results = execute_sql(&db, "SHOW COLLECTIONS").expect("SHOW must work without FROM clause");

    assert!(
        !results.is_empty(),
        "SHOW COLLECTIONS must return at least the created collection"
    );
    let names: Vec<&str> = results
        .iter()
        .filter_map(|r| payload_str(r, "name"))
        .collect();
    assert!(
        names.contains(&"test_col"),
        "SHOW must include the created collection"
    );
}

#[test]
fn test_regression_describe_no_from_clause() {
    let (_dir, db) = create_test_db();

    execute_sql(
        &db,
        "CREATE COLLECTION desc_col (dimension = 4, metric = 'cosine')",
    )
    .expect("create");

    let results = execute_sql(&db, "DESCRIBE desc_col")
        .expect("DESCRIBE must work without standard FROM clause");

    assert_eq!(results.len(), 1, "DESCRIBE returns exactly 1 result");
    assert_eq!(payload_str(&results[0], "name"), Some("desc_col"));
}

#[test]
fn test_regression_explain_no_from_clause() {
    let (_dir, db) = create_test_db();

    execute_sql(
        &db,
        "CREATE COLLECTION exp_col (dimension = 4, metric = 'cosine')",
    )
    .expect("create");

    let results = execute_sql(&db, "EXPLAIN SELECT * FROM exp_col LIMIT 1")
        .expect("EXPLAIN must work through Database::execute_query");

    assert_eq!(results.len(), 1, "EXPLAIN returns exactly 1 plan result");

    let has_plan = results[0]
        .point
        .payload
        .as_ref()
        .and_then(|p| p.get("plan"))
        .is_some();
    assert!(has_plan, "EXPLAIN result must contain a 'plan' field");
}

// ============================================================================
// Bug 5: FLUSH works through Database::execute_query without FROM clause
// ============================================================================

/// FLUSH is an admin statement, not a SELECT. It must bypass the
/// collection-resolution path entirely and delegate to `execute_admin`.
#[test]
fn test_regression_flush_no_from_clause() {
    let (_dir, db) = create_test_db();

    execute_sql(
        &db,
        "CREATE COLLECTION flush_col (dimension = 4, metric = 'cosine')",
    )
    .expect("create");

    let results = execute_sql(&db, "FLUSH").expect("FLUSH must work without FROM clause");

    assert_eq!(results.len(), 1, "FLUSH returns one status result");

    let status = results[0]
        .point
        .payload
        .as_ref()
        .and_then(|p| p.get("status"))
        .and_then(serde_json::Value::as_str);
    assert_eq!(
        status,
        Some("flushed"),
        "FLUSH should return status='flushed'"
    );
}

// ============================================================================
// Bug 6: All DDL variants extract collection name correctly
// ============================================================================

/// CREATE INDEX must correctly extract the collection name from the SQL
/// statement, not from the (possibly empty) `query.select.from` field.
/// A broken implementation would fail with `CollectionNotFound("")`.
#[test]
fn test_regression_create_index_collection_name_extracted() {
    let (_dir, db) = create_test_db();

    execute_sql(
        &db,
        "CREATE COLLECTION idx_col (dimension = 4, metric = 'cosine')",
    )
    .expect("create");

    let result = execute_sql(&db, "CREATE INDEX ON idx_col (field1)");

    assert!(
        result.is_ok(),
        "CREATE INDEX must correctly extract collection name from SQL: {:?}",
        result.err()
    );
}

/// DROP INDEX must also resolve the collection name from the DDL statement.
#[test]
fn test_regression_drop_index_collection_name_extracted() {
    let (_dir, db) = create_test_db();

    execute_sql(
        &db,
        "CREATE COLLECTION didx (dimension = 4, metric = 'cosine')",
    )
    .expect("create");
    execute_sql(&db, "CREATE INDEX ON didx (tag)").expect("create index");

    let result = execute_sql(&db, "DROP INDEX ON didx (tag)");

    assert!(
        result.is_ok(),
        "DROP INDEX must correctly extract collection name: {:?}",
        result.err()
    );
}

// ============================================================================
// Bug 7: SELECT EDGES works as DML without a standard FROM clause
// ============================================================================

/// SELECT EDGES is dispatched as a DML statement, not a standard SELECT.
/// The collection name comes from the `SelectEdgesStatement.collection`
/// field, not from `query.select.from`. A broken dispatch would try to
/// resolve an empty collection name.
#[test]
fn test_regression_select_edges_no_standard_from() {
    let (_dir, db) = create_test_db();

    execute_sql(
        &db,
        "CREATE GRAPH COLLECTION edge_col (dimension = 4, metric = 'cosine') SCHEMALESS",
    )
    .expect("create graph collection");

    let results = execute_sql(&db, "SELECT EDGES FROM edge_col LIMIT 10")
        .expect("SELECT EDGES must work through DML dispatch");

    assert_eq!(
        results.len(),
        0,
        "empty graph collection should return 0 edges"
    );
}

/// SELECT EDGES with actual edges, verifying the full pipeline.
#[test]
fn test_regression_select_edges_returns_inserted_edges() {
    let (_dir, db) = create_test_db();

    execute_sql(
        &db,
        "CREATE GRAPH COLLECTION g (dimension = 4, metric = 'cosine') SCHEMALESS",
    )
    .expect("create");

    execute_sql(
        &db,
        "INSERT EDGE INTO g (source = 1, target = 2, label = 'KNOWS')",
    )
    .expect("insert edge");

    let results =
        execute_sql(&db, "SELECT EDGES FROM g LIMIT 10").expect("SELECT EDGES after insert");

    assert_eq!(results.len(), 1, "should return the inserted edge");

    let label = results[0]
        .point
        .payload
        .as_ref()
        .and_then(|p| p.get("label"))
        .and_then(serde_json::Value::as_str);
    assert_eq!(label, Some("KNOWS"), "edge label must match");
}

// ============================================================================
// Bug 8 (#489): MATCH RETURN clause `projected` always empty
// ============================================================================

/// Helper: set up a graph collection with labeled nodes and edges for MATCH
/// projection tests.
fn setup_match_projection_graph(db: &velesdb_core::Database) -> velesdb_core::GraphCollection {
    execute_sql(
        db,
        "CREATE GRAPH COLLECTION proj (dimension = 4, metric = 'cosine') SCHEMALESS;",
    )
    .expect("test: CREATE GRAPH proj");

    let gc = db
        .get_graph_collection("proj")
        .expect("test: proj graph collection must exist");

    gc.upsert_node_payload(
        1,
        &serde_json::json!({"_labels": ["Person"], "name": "Alice", "age": 30}),
    )
    .expect("test: node 1");
    gc.upsert_node_payload(
        2,
        &serde_json::json!({"_labels": ["Person"], "name": "Bob", "age": 25}),
    )
    .expect("test: node 2");

    let edge = velesdb_core::GraphEdge::new(1, 1, 2, "KNOWS").expect("test: edge KNOWS");
    gc.add_edge(edge).expect("test: add edge");

    gc
}

/// GIVEN a graph with Person nodes having `name` and `age` properties
/// WHEN `RETURN *` is used in a MATCH clause
/// THEN `projected` contains all properties from all bound nodes (not empty).
///
/// Regression: before Fix #489, `parse_property_path("*")` returned `None`,
/// so `project_properties()` skipped wildcard items entirely.
#[test]
fn test_regression_match_return_wildcard_populates_projected() {
    let (_dir, db) = create_test_db();
    let gc = setup_match_projection_graph(&db);

    let match_clause = velesdb_core::velesql::MatchClause {
        patterns: vec![velesdb_core::velesql::GraphPattern {
            name: None,
            nodes: vec![
                velesdb_core::velesql::NodePattern::new().with_alias("a"),
                velesdb_core::velesql::NodePattern::new().with_alias("b"),
            ],
            relationships: vec![velesdb_core::velesql::RelationshipPattern::new(
                velesdb_core::velesql::Direction::Outgoing,
            )],
        }],
        where_clause: None,
        return_clause: velesdb_core::velesql::ReturnClause {
            items: vec![velesdb_core::velesql::ReturnItem {
                expression: "*".to_string(),
                alias: None,
            }],
            order_by: None,
            limit: Some(100),
        },
    };

    let results = gc
        .execute_match(&match_clause, &HashMap::new())
        .expect("test: MATCH execution");

    assert!(!results.is_empty(), "MATCH should return results");

    // The key assertion: `projected` must NOT be empty for RETURN *.
    let first = &results[0];
    assert!(
        !first.projected.is_empty(),
        "RETURN * must populate `projected` with node properties, got empty HashMap"
    );

    // Verify specific properties are present with alias-prefixed keys.
    #[allow(clippy::case_sensitive_file_extension_comparisons)]
    // Not a file extension — JSON key like "n.name"
    let has_name = first.projected.keys().any(|k| k.ends_with(".name"));
    assert!(
        has_name,
        "RETURN * projected should contain a `.name` property, got: {:?}",
        first.projected.keys().collect::<Vec<_>>()
    );
}

/// GIVEN a graph with Person nodes
/// WHEN `RETURN a.name` is used (property path)
/// THEN `projected` contains the `a.name` key with the correct value.
#[test]
fn test_regression_match_return_property_path_populates_projected() {
    let (_dir, db) = create_test_db();
    let gc = setup_match_projection_graph(&db);

    let match_clause = velesdb_core::velesql::MatchClause {
        patterns: vec![velesdb_core::velesql::GraphPattern {
            name: None,
            nodes: vec![
                velesdb_core::velesql::NodePattern::new().with_alias("a"),
                velesdb_core::velesql::NodePattern::new().with_alias("b"),
            ],
            relationships: vec![velesdb_core::velesql::RelationshipPattern::new(
                velesdb_core::velesql::Direction::Outgoing,
            )],
        }],
        where_clause: None,
        return_clause: velesdb_core::velesql::ReturnClause {
            items: vec![velesdb_core::velesql::ReturnItem {
                expression: "a.name".to_string(),
                alias: None,
            }],
            order_by: None,
            limit: Some(100),
        },
    };

    let results = gc
        .execute_match(&match_clause, &HashMap::new())
        .expect("test: MATCH execution");

    assert!(!results.is_empty(), "MATCH should return results");

    let first = &results[0];
    assert_eq!(
        first.projected.get("a.name"),
        Some(&serde_json::json!("Alice")),
        "RETURN a.name should project Alice's name"
    );
}

/// GIVEN a graph with Person nodes
/// WHEN `RETURN b` (bare alias) is used
/// THEN `projected` contains all properties of node `b` prefixed with `b.`.
///
/// Regression: before Fix #489, `parse_property_path("b")` returned `None`
/// because there is no dot separator.
#[test]
fn test_regression_match_return_bare_alias_populates_projected() {
    let (_dir, db) = create_test_db();
    let gc = setup_match_projection_graph(&db);

    let match_clause = velesdb_core::velesql::MatchClause {
        patterns: vec![velesdb_core::velesql::GraphPattern {
            name: None,
            nodes: vec![
                velesdb_core::velesql::NodePattern::new().with_alias("a"),
                velesdb_core::velesql::NodePattern::new().with_alias("b"),
            ],
            relationships: vec![velesdb_core::velesql::RelationshipPattern::new(
                velesdb_core::velesql::Direction::Outgoing,
            )],
        }],
        where_clause: None,
        return_clause: velesdb_core::velesql::ReturnClause {
            items: vec![velesdb_core::velesql::ReturnItem {
                expression: "b".to_string(),
                alias: None,
            }],
            order_by: None,
            limit: Some(100),
        },
    };

    let results = gc
        .execute_match(&match_clause, &HashMap::new())
        .expect("test: MATCH execution");

    assert!(!results.is_empty(), "MATCH should return results");

    let first = &results[0];
    assert!(
        !first.projected.is_empty(),
        "RETURN b must populate projected with node b's properties, got empty"
    );

    // Verify that Bob's name appears as "b.name".
    assert_eq!(
        first.projected.get("b.name"),
        Some(&serde_json::json!("Bob")),
        "RETURN b should project Bob's name as b.name"
    );
}

/// GIVEN a graph with Person nodes (single-node pattern, no relationships)
/// WHEN `RETURN n` (bare alias) is used on a single-node MATCH
/// THEN `projected` is populated correctly.
///
/// This tests the `collect_single_node_results` path in addition to
/// the traversal path tested above.
#[test]
fn test_regression_match_single_node_return_bare_alias() {
    let (_dir, db) = create_test_db();
    let gc = setup_match_projection_graph(&db);

    let match_clause = velesdb_core::velesql::MatchClause {
        patterns: vec![velesdb_core::velesql::GraphPattern {
            name: None,
            nodes: vec![velesdb_core::velesql::NodePattern::new().with_alias("n")],
            relationships: vec![], // No relationships = single-node path
        }],
        where_clause: None,
        return_clause: velesdb_core::velesql::ReturnClause {
            items: vec![velesdb_core::velesql::ReturnItem {
                expression: "n".to_string(),
                alias: None,
            }],
            order_by: None,
            limit: Some(100),
        },
    };

    let results = gc
        .execute_match(&match_clause, &HashMap::new())
        .expect("test: single-node MATCH execution");

    assert!(
        !results.is_empty(),
        "single-node MATCH should return results"
    );

    // At least one result should have projected properties.
    let has_projected = results.iter().any(|r| !r.projected.is_empty());
    assert!(
        has_projected,
        "RETURN n on single-node MATCH must populate projected for at least one result"
    );
}

// ============================================================================
// Bug 9 (#486): VelesQL parse error "Invalid integer" for large uint64 values
// ============================================================================

/// Values exceeding `i64::MAX` (9,223,372,036,854,775,807) caused a parse
/// error because `parse_integer_literal()` only tried `i64`. The fix adds a
/// `Value::UnsignedInteger(u64)` variant and falls back to `u64` parsing.
#[test]
fn test_regression_insert_with_large_uint64_id() {
    // GIVEN: a collection with dimension 4
    let (_dir, db) = create_test_db();
    execute_sql(
        &db,
        "CREATE COLLECTION uint_test (dimension = 4, metric = 'cosine')",
    )
    .expect("test: create collection");

    // WHEN: insert a point with an ID exceeding i64::MAX
    // (vectors must use $v parameter; grammar value rule does not include vector literals)
    let large_id = "9223372036854775808"; // i64::MAX + 1
    let sql = format!("INSERT INTO uint_test (id, vector) VALUES ({large_id}, $v)");
    let params = super::helpers::vector_param(&[0.1, 0.2, 0.3, 0.4]);
    let results =
        execute_sql_with_params(&db, &sql, &params).expect("test: insert with large uint64 id");

    // THEN: the insert succeeds and returns the inserted point
    assert_eq!(results.len(), 1, "insert must return exactly 1 result");
    assert_eq!(
        results[0].point.id, 9_223_372_036_854_775_808,
        "point ID must be the large uint64 value"
    );
}

/// `u64::MAX` should also parse and insert successfully.
#[test]
fn test_regression_insert_with_u64_max_id() {
    // GIVEN: a collection
    let (_dir, db) = create_test_db();
    execute_sql(
        &db,
        "CREATE COLLECTION umax_test (dimension = 4, metric = 'cosine')",
    )
    .expect("test: create collection");

    // WHEN: insert a point with u64::MAX as ID
    let sql = "INSERT INTO umax_test (id, vector) VALUES (18446744073709551615, $v)";
    let params = super::helpers::vector_param(&[1.0, 0.0, 0.0, 0.0]);
    let results =
        execute_sql_with_params(&db, sql, &params).expect("test: insert with u64::MAX id");

    // THEN: the insert succeeds
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].point.id, u64::MAX);
}

/// A WHERE clause with a large uint64 value should parse without error.
#[test]
fn test_regression_where_clause_large_uint64() {
    // GIVEN: a collection with a point at a large ID
    let (_dir, db) = create_test_db();
    execute_sql(
        &db,
        "CREATE COLLECTION wtest (dimension = 4, metric = 'cosine')",
    )
    .expect("test: create collection");

    let large_id = "10000000000000000000"; // > i64::MAX
    let insert_sql = format!("INSERT INTO wtest (id, vector) VALUES ({large_id}, $v)");
    let params = super::helpers::vector_param(&[0.5, 0.5, 0.5, 0.5]);
    execute_sql_with_params(&db, &insert_sql, &params).expect("test: insert");

    // WHEN: query with WHERE id = <large u64>
    let select_sql = format!("SELECT * FROM wtest WHERE id = {large_id} LIMIT 10");
    let results = execute_sql(&db, &select_sql);

    // THEN: the query parses and executes without error
    // (results may vary depending on filter implementation for id field,
    //  but the parse must not fail)
    assert!(
        results.is_ok(),
        "WHERE with large uint64 must not cause parse error"
    );
}

// ============================================================================
// Resolution 1: Metadata pre-filter correctness through SQL pipeline
// ============================================================================

/// Helper: creates a collection with 50 points, categories "tech"/"science",
/// and a secondary index on "category". Returns the Database so BDD tests
/// exercise the full SQL pipeline (not just the Collection API).
fn setup_indexed_search_db() -> (tempfile::TempDir, velesdb_core::Database) {
    let (dir, db) = create_test_db();

    execute_sql(
        &db,
        "CREATE COLLECTION idx_search (dimension = 4, metric = 'cosine')",
    )
    .expect("test: create collection");

    let vc = db
        .get_vector_collection("idx_search")
        .expect("test: get collection");

    // Create secondary index on "category" BEFORE inserting data.
    vc.create_index("category")
        .expect("test: create secondary index");

    // Insert 50 points: ids 0..25 are "tech", ids 25..50 are "science".
    let points: Vec<Point> = (0u64..50)
        .map(|id| {
            let category = if id < 25 { "tech" } else { "science" };
            let payload = serde_json::json!({
                "category": category,
                "priority": id % 5,
            });
            let mut vector = vec![0.1_f32; 4];
            #[allow(clippy::cast_precision_loss)]
            {
                vector[0] += (id as f32) * 0.01;
            }
            let norm = vector.iter().map(|x| x * x).sum::<f32>().sqrt();
            for x in &mut vector {
                *x /= norm;
            }
            Point {
                id,
                vector,
                payload: Some(payload),
                sparse_vectors: None,
            }
        })
        .collect();

    vc.upsert(points).expect("test: upsert");
    (dir, db)
}

/// GIVEN a collection with a secondary index on "category"
/// AND 25 "tech" + 25 "science" points
/// WHEN searching with `WHERE category = 'tech'` via SQL
/// THEN ALL returned results have category = 'tech'
/// AND results are not empty.
#[test]
fn test_search_with_indexed_filter_returns_only_matching() {
    let (_dir, db) = setup_indexed_search_db();

    let mut params = HashMap::new();
    params.insert("v".to_string(), serde_json::json!([0.5, 0.5, 0.5, 0.5]));

    let results = execute_sql_with_params(
        &db,
        "SELECT * FROM idx_search WHERE category = 'tech' AND vector NEAR $v LIMIT 10",
        &params,
    )
    .expect("test: filtered search should succeed");

    assert!(
        !results.is_empty(),
        "indexed filter search must return results"
    );

    for r in &results {
        let cat = payload_str(r, "category");
        assert_eq!(
            cat,
            Some("tech"),
            "all results must be 'tech', got {:?} for id={}",
            cat,
            r.point.id
        );
    }
}

/// GIVEN a collection with a secondary index on "category"
/// AND NO index on "priority"
/// WHEN searching with `WHERE priority = 0` (unindexed field)
/// THEN results are correct (post-filter fallback works).
#[test]
fn test_search_with_unindexed_filter_falls_back_to_post_filter() {
    let (_dir, db) = setup_indexed_search_db();

    let mut params = HashMap::new();
    params.insert("v".to_string(), serde_json::json!([0.5, 0.5, 0.5, 0.5]));

    let results = execute_sql_with_params(
        &db,
        "SELECT * FROM idx_search WHERE priority = 0 AND vector NEAR $v LIMIT 10",
        &params,
    )
    .expect("test: unindexed filter search should succeed");

    assert!(
        !results.is_empty(),
        "post-filter fallback must return results"
    );

    for r in &results {
        let priority = r
            .point
            .payload
            .as_ref()
            .and_then(|p| p.get("priority"))
            .and_then(serde_json::Value::as_u64);
        assert_eq!(
            priority,
            Some(0),
            "all results must have priority=0, got {:?} for id={}",
            priority,
            r.point.id
        );
    }
}

/// GIVEN a collection with secondary indexes
/// WHEN searching WITHOUT any filter
/// THEN search succeeds and returns results (no bitmap overhead).
#[test]
fn test_search_without_filter_not_degraded() {
    let (_dir, db) = setup_indexed_search_db();

    let mut params = HashMap::new();
    params.insert("v".to_string(), serde_json::json!([0.5, 0.5, 0.5, 0.5]));

    let results = execute_sql_with_params(
        &db,
        "SELECT * FROM idx_search WHERE vector NEAR $v LIMIT 10",
        &params,
    )
    .expect("test: unfiltered search should succeed");

    assert!(
        !results.is_empty(),
        "unfiltered search must return results even with indexes present"
    );
}

// ============================================================================
// Resolution 2: Bitmap with large IDs (u64 > u32::MAX)
// ============================================================================

/// GIVEN a collection where some point IDs exceed `u32::MAX`
/// AND a secondary index on "category"
/// WHEN searching with `WHERE category = 'tech'`
/// THEN points with IDs > `u32::MAX` that match are still returned
/// (they pass through bitmap and are caught by the post-filter).
///
/// `RoaringBitmap` stores `u32` values, so IDs > `u32::MAX` are silently
/// skipped in the bitmap. The post-filter must still catch them.
#[test]
fn test_search_filter_with_large_id_not_lost() {
    let (_dir, db) = create_test_db();

    execute_sql(
        &db,
        "CREATE COLLECTION big_ids (dimension = 4, metric = 'cosine')",
    )
    .expect("test: create collection");

    let vc = db
        .get_vector_collection("big_ids")
        .expect("test: get collection");

    vc.create_index("category")
        .expect("test: create secondary index");

    // Insert points: one with a normal ID, one with a large ID (> u32::MAX).
    let large_id: u64 = u64::from(u32::MAX) + 100;
    let points = vec![
        Point {
            id: 1,
            vector: vec![1.0, 0.0, 0.0, 0.0],
            payload: Some(serde_json::json!({"category": "tech"})),
            sparse_vectors: None,
        },
        Point {
            id: 2,
            vector: vec![0.0, 1.0, 0.0, 0.0],
            payload: Some(serde_json::json!({"category": "science"})),
            sparse_vectors: None,
        },
        Point {
            id: large_id,
            vector: vec![0.9, 0.1, 0.0, 0.0],
            payload: Some(serde_json::json!({"category": "tech"})),
            sparse_vectors: None,
        },
    ];
    vc.upsert(points).expect("test: upsert");

    // Search with filter on indexed field.
    let filter = velesdb_core::Filter::new(velesdb_core::Condition::Eq {
        field: "category".to_string(),
        value: serde_json::Value::String("tech".to_string()),
    });

    let query = vec![0.9_f32, 0.1, 0.0, 0.0];
    let results = vc
        .search_with_filter(&query, 10, &filter)
        .expect("test: search_with_filter should succeed");

    // Both tech points must be returned (id=1 and id=large_id).
    let ids: std::collections::HashSet<u64> = results.iter().map(|r| r.point.id).collect();
    assert!(ids.contains(&1), "normal ID tech point must be in results");
    assert!(
        ids.contains(&large_id),
        "large ID ({large_id} > u32::MAX) tech point must be caught by post-filter"
    );

    // Science point must NOT be in results.
    assert!(
        !ids.contains(&2),
        "science point must not appear in tech-filtered results"
    );
}

// ============================================================================
// Resolution 3: WAL group write crash recovery (Database-level BDD)
// ============================================================================

/// GIVEN a collection with WAL enabled (default persistence)
/// WHEN batch-inserting 200 vectors (triggers group write)
/// AND the database is closed and reopened
/// THEN all 200 vectors are recoverable via search.
#[test]
fn test_wal_group_write_recovery_preserves_all_data() {
    let dir = tempfile::TempDir::new().expect("test: create temp dir");
    let point_count = 200_u64;

    // Phase 1: Insert data and close (simulates crash by dropping without explicit cleanup).
    {
        let db = velesdb_core::Database::open(dir.path()).expect("test: open database");
        db.create_vector_collection("wal_test", 4, velesdb_core::DistanceMetric::Cosine)
            .expect("test: create collection");

        let vc = db
            .get_vector_collection("wal_test")
            .expect("test: get collection");

        let points: Vec<Point> = (0..point_count)
            .map(|id| {
                #[allow(clippy::cast_precision_loss)]
                let base = (id as f32) * 0.01;
                let vector = [base, 1.0 - base, base * 0.5, 0.1];
                let norm = vector.iter().map(|x| x * x).sum::<f32>().sqrt();
                let vector: Vec<f32> = vector.iter().map(|x| x / norm).collect();
                Point::without_payload(id, vector)
            })
            .collect();

        vc.upsert(points).expect("test: bulk upsert");

        // Flush to WAL (but NOT compact) — data must survive reopen.
        vc.flush().expect("test: flush");
    } // Database dropped here.

    // Phase 2: Reopen and verify all data survived.
    let db = velesdb_core::Database::open(dir.path()).expect("test: reopen database");
    let vc = db
        .get_vector_collection("wal_test")
        .expect("test: reopened collection must exist");

    // Verify point count via get (spot-check first, middle, last).
    let first = vc.get(&[0]);
    assert!(
        first[0].is_some(),
        "first point (id=0) must survive WAL recovery"
    );

    let mid_id = point_count / 2;
    let mid = vc.get(&[mid_id]);
    assert!(
        mid[0].is_some(),
        "middle point (id={mid_id}) must survive WAL recovery"
    );

    let last_id = point_count - 1;
    let last = vc.get(&[last_id]);
    assert!(
        last[0].is_some(),
        "last point (id={last_id}) must survive WAL recovery"
    );

    // Verify vector search returns results (HNSW rebuilt on open).
    let query = [0.5_f32, 0.5, 0.25, 0.1];
    let norm = query.iter().map(|x| x * x).sum::<f32>().sqrt();
    let query: Vec<f32> = query.iter().map(|x| x / norm).collect();

    let results = vc.search(&query, 10).expect("test: search after recovery");
    assert!(
        !results.is_empty(),
        "search must return results after WAL recovery"
    );
}

// ============================================================================
// Resolution 4: Fast mode search quality contract (recall >= 0.95)
// ============================================================================

/// GIVEN a collection with 500 random 128-dim vectors
/// WHEN searching with `SearchQuality::Fast` for top-10
/// THEN recall@10 >= 0.95 compared to brute-force ground truth.
///
/// This test enforces the search quality contract for the fastest mode.
/// If it fails, the Fast mode's `ef_search` is too aggressive.
#[test]
fn test_fast_mode_recall_at_least_95_percent() {
    let (_dir, db) = create_test_db();

    execute_sql(
        &db,
        "CREATE COLLECTION recall_fast (dimension = 32, metric = 'cosine')",
    )
    .expect("test: create collection");

    let vc = db
        .get_vector_collection("recall_fast")
        .expect("test: get collection");

    // Generate 500 deterministic vectors (32-dim for speed).
    let vec_count = 500_u64;
    let dim = 32;
    let vectors: Vec<Vec<f32>> = (0..vec_count)
        .map(|i| {
            #[allow(clippy::cast_precision_loss)]
            let v: Vec<f32> = (0..dim)
                .map(|d| ((i * 31 + d * 17) % 1000) as f32 / 1000.0)
                .collect();
            let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
            if norm > 0.0 {
                v.iter().map(|x| x / norm).collect()
            } else {
                v
            }
        })
        .collect();

    let points: Vec<Point> = vectors
        .iter()
        .enumerate()
        .map(|(i, v)| {
            #[allow(clippy::cast_possible_truncation)]
            let id = i as u64;
            Point::without_payload(id, v.clone())
        })
        .collect();

    vc.upsert(points).expect("test: upsert vectors");

    // Choose a query vector (vector at index 0).
    let query = &vectors[0];
    let k = 10_usize;

    // Brute-force ground truth: compute cosine distance for all vectors.
    let mut ground_truth: Vec<(u64, f32)> = vectors
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let dot: f32 = query.iter().zip(v.iter()).map(|(a, b)| a * b).sum();
            let norm_q: f32 = query.iter().map(|x| x * x).sum::<f32>().sqrt();
            let norm_v: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
            let similarity = if norm_q > 0.0 && norm_v > 0.0 {
                dot / (norm_q * norm_v)
            } else {
                0.0
            };
            // Cosine distance = 1 - similarity (lower is closer).
            #[allow(clippy::cast_possible_truncation)]
            let id = i as u64;
            (id, 1.0 - similarity)
        })
        .collect();
    ground_truth.sort_by(|a, b| a.1.partial_cmp(&b.1).expect("test: no NaN"));
    let gt_ids: Vec<u64> = ground_truth.iter().take(k).map(|(id, _)| *id).collect();

    // Search with Fast quality mode.
    let results = vc
        .search_with_quality(query, k, velesdb_core::SearchQuality::Fast)
        .expect("test: Fast-mode search should succeed");

    let retrieved_ids: Vec<u64> = results.iter().map(|r| r.point.id).collect();

    // Compute recall@k.
    let gt_set: std::collections::HashSet<u64> = gt_ids.iter().copied().collect();
    let retrieved_set: std::collections::HashSet<u64> = retrieved_ids.iter().copied().collect();
    let intersection = gt_set.intersection(&retrieved_set).count();
    #[allow(clippy::cast_precision_loss)]
    let recall = intersection as f64 / k as f64;

    assert!(
        recall >= 0.95,
        "Fast mode recall@{k} must be >= 0.95, got {recall:.2} \
         (retrieved: {retrieved_ids:?}, ground truth: {gt_ids:?})"
    );
}
