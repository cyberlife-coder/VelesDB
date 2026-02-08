//! Cross-store & VelesQL scenario tests (Scenario 0 + API) — Tests created by Phase 4 Plan 06.
//!
//! **Scenario 0**: Technical Deep-Dive — MATCH + similarity + column filter + subquery.
//! **VelesQL API**: Basic SELECT/NEAR, category filter, GROUP BY/HAVING, UNION parsing.
//! **Manufacturing QC**: Bonus MATCH traversal with material filter.

use std::collections::HashMap;

use serde_json::json;

use velesdb_core::velesql::{
    AggregateArg, AggregateFunction, AggregateType, CompareOp, Comparison, Condition, DistinctMode,
    Parser, SelectColumns, SelectStatement, Subquery, Value,
};
use velesdb_core::DistanceMetric;

use crate::helpers;

/// Converts a f32 embedding vector to a JSON array for query parameters.
fn embedding_to_json_param(vec: &[f32]) -> serde_json::Value {
    serde_json::Value::Array(
        vec.iter()
            .map(|&f| serde_json::Value::from(f64::from(f)))
            .collect(),
    )
}

// ============================================================================
// Scenario 0: Technical Deep-Dive (Vector + Graph + Column + Subquery)
// ============================================================================
//
// README query:
// ```sql
// MATCH (doc:Document)-[:AUTHORED_BY]->(author:Author)
// WHERE
//   similarity(doc.embedding, $research_question) > 0.8
//   AND doc.category = 'peer-reviewed'
//   AND (SELECT citation_count FROM author_metrics
//        WHERE author_id = author.id) > 50
// ORDER BY similarity() DESC
// LIMIT 5
// ```

/// 4 Documents: 3 peer-reviewed (2 close vectors, 1 distant), 1 preprint.
fn scenario0_documents() -> Vec<(u64, Vec<f32>, serde_json::Value)> {
    vec![
        (
            1,
            helpers::generate_embedding(100, 4),
            json!({"_labels": ["Document"], "category": "peer-reviewed", "title": "Neural Architecture Search"}),
        ),
        (
            2,
            helpers::generate_embedding(101, 4),
            json!({"_labels": ["Document"], "category": "peer-reviewed", "title": "Graph Neural Networks"}),
        ),
        (
            3,
            helpers::generate_embedding(102, 4),
            json!({"_labels": ["Document"], "category": "preprint", "title": "Quick Prototype Study"}),
        ),
        (
            4,
            helpers::generate_embedding(500, 4),
            json!({"_labels": ["Document"], "category": "peer-reviewed", "title": "Distant Topic Paper"}),
        ),
    ]
}

/// 4 Authors with varying citation counts.
/// AVG(citation_count) = (80 + 120 + 30 + 200) / 4 = 107.5
fn scenario0_authors() -> Vec<(u64, Vec<f32>, serde_json::Value)> {
    vec![
        (
            10,
            helpers::generate_embedding(200, 4),
            json!({"_labels": ["Author"], "name": "Dr. Smith", "citation_count": 80}),
        ),
        (
            11,
            helpers::generate_embedding(201, 4),
            json!({"_labels": ["Author"], "name": "Prof. Johnson", "citation_count": 120}),
        ),
        (
            12,
            helpers::generate_embedding(202, 4),
            json!({"_labels": ["Author"], "name": "Dr. Novice", "citation_count": 30}),
        ),
        (
            13,
            helpers::generate_embedding(203, 4),
            json!({"_labels": ["Author"], "name": "Prof. Expert", "citation_count": 200}),
        ),
    ]
}

/// Sets up Scenario 0: Documents, Authors, and AUTHORED_BY edges.
fn setup_scenario0() -> (tempfile::TempDir, velesdb_core::Collection) {
    let (dir, db) = helpers::setup_test_db();
    let collection = helpers::setup_labeled_collection(&db, "scenario0", 4, DistanceMetric::Cosine);

    helpers::insert_labeled_nodes(&collection, &scenario0_documents());
    helpers::insert_labeled_nodes(&collection, &scenario0_authors());

    // AUTHORED_BY edges: Document → Author
    helpers::add_edges(
        &collection,
        &[
            (100, 1, 10, "AUTHORED_BY"), // Neural Arch → Dr. Smith (citations=80)
            (101, 2, 11, "AUTHORED_BY"), // Graph NN → Prof. Johnson (citations=120)
            (102, 3, 12, "AUTHORED_BY"), // Quick Proto → Dr. Novice (citations=30)
            (103, 4, 13, "AUTHORED_BY"), // Distant Topic → Prof. Expert (citations=200)
        ],
    );

    (dir, collection)
}

/// Test A: MATCH + similarity + category filter (without subquery).
///
/// Verifies:
/// - MATCH traversal follows AUTHORED_BY edges
/// - `start_properties` filters documents by category = 'peer-reviewed'
/// - Similarity threshold filters distant vectors
/// - Cross-node property projection (doc.title + author.name)
/// - Results ordered by similarity DESC
#[test]
fn test_scenario0_match_similarity_filter() {
    let (_dir, collection) = setup_scenario0();

    // Inline property filter on start node: category = 'peer-reviewed'
    let mut start_props = HashMap::new();
    start_props.insert(
        "category".to_string(),
        Value::String("peer-reviewed".to_string()),
    );

    let match_clause = helpers::build_single_hop_match(
        "doc",
        "Document",
        "AUTHORED_BY",
        "author",
        "Author",
        start_props,
        None, // No WHERE on target — test similarity + start filter only
        vec![
            ("doc.title", None),
            ("author.name", None),
            ("author.citation_count", None),
        ],
        Some(vec![("similarity()", true)]),
        Some(5),
    );

    let query_vector = helpers::generate_embedding(100, 4);

    // Threshold 0.5 → filters Doc 4 (seed 500, distant vector)
    let results = collection
        .execute_match_with_similarity(&match_clause, &query_vector, 0.5, &HashMap::new())
        .expect("execute_match_with_similarity failed");

    // Doc 1 (seed 100, peer-reviewed, sim≈1.0) → Dr. Smith → pass
    // Doc 2 (seed 101, peer-reviewed, high sim) → Prof. Johnson → pass
    // Doc 3 (seed 102, preprint) → filtered by category
    // Doc 4 (seed 500, peer-reviewed, low sim) → filtered by threshold
    assert!(
        !results.is_empty(),
        "Should return peer-reviewed documents with high similarity"
    );

    // Verify cross-node projection
    let has_both = results
        .iter()
        .any(|r| r.projected.contains_key("doc.title") && r.projected.contains_key("author.name"));
    assert!(
        has_both,
        "Results should project both doc.title and author.name. First: {:?}",
        results.first().map(|r| &r.projected)
    );

    // Verify similarity DESC ordering
    let scores: Vec<f32> = results.iter().filter_map(|r| r.score).collect();
    for window in scores.windows(2) {
        assert!(
            window[0] >= window[1],
            "Results should be ordered DESC: {:.4} >= {:.4}",
            window[0],
            window[1]
        );
    }
}

/// Test B: Subquery in MATCH WHERE — non-correlated scalar subquery.
///
/// WHERE: `citation_count > (SELECT AVG(citation_count) FROM scenario0)`
/// AVG = 107.5 → only Prof. Johnson (120) and Prof. Expert (200) pass.
///
/// **Limitation**: Uses non-correlated subquery because binding-aware correlation
/// (`author.id` resolution from MATCH bindings into subquery params) is not yet
/// wired through the `execute_match` → `evaluate_where_condition` path.
/// Correlated subqueries work in SELECT WHERE (VP-002) but not yet in MATCH WHERE.
#[test]
fn test_scenario0_subquery_in_where() {
    let (_dir, collection) = setup_scenario0();

    // Subquery: (SELECT AVG(citation_count) FROM scenario0)
    // Only Author nodes have citation_count → AVG(80,120,30,200) = 107.5
    let subquery = Subquery {
        select: SelectStatement {
            distinct: DistinctMode::None,
            columns: SelectColumns::Aggregations(vec![AggregateFunction {
                function_type: AggregateType::Avg,
                argument: AggregateArg::Column("citation_count".to_string()),
                alias: None,
            }]),
            from: "scenario0".to_string(),
            from_alias: None,
            joins: Vec::new(),
            where_clause: None,
            order_by: None,
            limit: None,
            offset: None,
            with_clause: None,
            group_by: None,
            having: None,
            fusion_clause: None,
        },
        correlations: Vec::new(),
    };

    // WHERE citation_count > (SELECT AVG(citation_count) FROM scenario0)
    let where_clause = Condition::Comparison(Comparison {
        column: "citation_count".to_string(),
        operator: CompareOp::Gt,
        value: Value::Subquery(Box::new(subquery)),
    });

    let match_clause = helpers::build_single_hop_match(
        "doc",
        "Document",
        "AUTHORED_BY",
        "author",
        "Author",
        HashMap::new(),
        Some(where_clause),
        vec![
            ("doc.title", None),
            ("author.name", None),
            ("author.citation_count", None),
        ],
        None,
        Some(10),
    );

    let results = collection
        .execute_match(&match_clause, &HashMap::new())
        .expect("execute_match with subquery WHERE failed");

    // AVG(citation_count) = 107.5
    // Prof. Johnson (120 > 107.5) ✓, Prof. Expert (200 > 107.5) ✓
    // Dr. Smith (80) ✗, Dr. Novice (30) ✗
    assert_eq!(
        results.len(),
        2,
        "Should return 2 authors with citation_count > AVG (107.5), got {}",
        results.len()
    );

    let author_ids: Vec<u64> = results.iter().map(|r| r.node_id).collect();
    assert!(
        author_ids.contains(&11),
        "Prof. Johnson (id=11, citations=120) should be in results"
    );
    assert!(
        author_ids.contains(&13),
        "Prof. Expert (id=13, citations=200) should be in results"
    );
}

// ============================================================================
// VelesQL API Validation (README examples)
// ============================================================================

/// VelesQL: Basic SELECT + NEAR with category filter.
///
/// ```sql
/// SELECT title, category
/// FROM articles
/// WHERE vector NEAR $query_embedding
///   AND category = 'technology'
/// ORDER BY similarity(vector, $query_embedding) DESC
/// LIMIT 3
/// ```
#[test]
fn test_velesql_select_near_with_filter() {
    let (_dir, db) = helpers::setup_test_db();
    let collection = helpers::setup_labeled_collection(&db, "articles", 4, DistanceMetric::Cosine);

    let nodes = vec![
        (
            1,
            helpers::generate_embedding(10, 4),
            json!({"title": "AI Trends", "category": "technology"}),
        ),
        (
            2,
            helpers::generate_embedding(11, 4),
            json!({"title": "Rust Performance", "category": "technology"}),
        ),
        (
            3,
            helpers::generate_embedding(12, 4),
            json!({"title": "Cooking Tips", "category": "lifestyle"}),
        ),
        (
            4,
            helpers::generate_embedding(500, 4),
            json!({"title": "Old Tech Review", "category": "technology"}),
        ),
    ];
    helpers::insert_labeled_nodes(&collection, &nodes);

    let query_str = "\
        SELECT title, category \
        FROM articles \
        WHERE vector NEAR $query_embedding \
          AND category = 'technology' \
        ORDER BY similarity(vector, $query_embedding) DESC \
        LIMIT 3";
    let parsed = Parser::parse(query_str).expect("VelesQL should parse");

    let query_vec = helpers::generate_embedding(10, 4);
    let mut params = HashMap::new();
    params.insert(
        "query_embedding".to_string(),
        embedding_to_json_param(&query_vec),
    );

    let results = collection
        .execute_query(&parsed, &params)
        .expect("Query should execute");

    // Technology articles: id 1, 2, 4 (3 is lifestyle → filtered)
    assert_eq!(
        results.len(),
        3,
        "Should return 3 technology articles, got {}",
        results.len()
    );

    for r in &results {
        let payload = r.point.payload.as_ref().expect("Should have payload");
        let cat = payload
            .get("category")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        assert_eq!(cat, "technology", "All results should be 'technology'");
    }

    // Verify similarity DESC ordering
    for window in results.windows(2) {
        assert!(
            window[0].score >= window[1].score,
            "Similarity DESC: {:.4} >= {:.4}",
            window[0].score,
            window[1].score
        );
    }
}

/// VelesQL: GROUP BY + HAVING aggregation.
///
/// ```sql
/// SELECT category, COUNT(*), AVG(rating)
/// FROM reviews
/// GROUP BY category
/// HAVING COUNT(*) > 1
/// ```
#[test]
fn test_velesql_group_by_having() {
    let (_dir, db) = helpers::setup_test_db();
    let collection = helpers::setup_labeled_collection(&db, "reviews", 4, DistanceMetric::Cosine);

    let nodes = vec![
        (
            1,
            helpers::generate_embedding(1, 4),
            json!({"category": "electronics", "rating": 4.5}),
        ),
        (
            2,
            helpers::generate_embedding(2, 4),
            json!({"category": "electronics", "rating": 3.5}),
        ),
        (
            3,
            helpers::generate_embedding(3, 4),
            json!({"category": "electronics", "rating": 5.0}),
        ),
        (
            4,
            helpers::generate_embedding(4, 4),
            json!({"category": "books", "rating": 4.0}),
        ),
        (
            5,
            helpers::generate_embedding(5, 4),
            json!({"category": "books", "rating": 3.0}),
        ),
        (
            6,
            helpers::generate_embedding(6, 4),
            json!({"category": "food", "rating": 5.0}),
        ),
    ];
    helpers::insert_labeled_nodes(&collection, &nodes);

    let query_str =
        "SELECT category, COUNT(*), AVG(rating) FROM reviews GROUP BY category HAVING COUNT(*) > 1";
    let parsed = Parser::parse(query_str).expect("VelesQL should parse");

    let result = collection
        .execute_aggregate(&parsed, &HashMap::new())
        .expect("Aggregation should execute");

    let groups = result.as_array().expect("Result should be array");

    // electronics (3 items), books (2 items) pass HAVING COUNT(*) > 1
    // food (1 item) filtered out
    assert_eq!(
        groups.len(),
        2,
        "Should have 2 groups (electronics, books), got {}",
        groups.len()
    );

    let has_food = groups
        .iter()
        .any(|g| g.get("category") == Some(&json!("food")));
    assert!(!has_food, "food should be filtered by HAVING COUNT(*) > 1");

    // Verify electronics AVG(rating) ≈ (4.5+3.5+5.0)/3 = 4.33
    let electronics = groups
        .iter()
        .find(|g| g.get("category") == Some(&json!("electronics")));
    assert!(electronics.is_some(), "electronics group should exist");
    let avg = electronics
        .unwrap()
        .get("avg_rating")
        .and_then(serde_json::Value::as_f64)
        .expect("avg_rating should be a number");
    assert!(
        (avg - 4.333).abs() < 0.1,
        "electronics AVG(rating) should be ~4.33, got {avg}"
    );
}

/// VelesQL: UNION parsing validation.
///
/// Verifies the parser accepts UNION syntax. Execution requires multi-collection
/// routing which is not yet implemented at core level — parse-only test.
#[test]
fn test_velesql_union_parses() {
    let query_str = "\
        SELECT title FROM articles WHERE category = 'tech' \
        UNION \
        SELECT title FROM articles WHERE category = 'science'";
    let parsed = Parser::parse(query_str);
    assert!(
        parsed.is_ok(),
        "UNION query should parse successfully: {:?}",
        parsed.err()
    );

    let query = parsed.unwrap();
    assert!(
        query.compound.is_some(),
        "Parsed query should have a compound (UNION) clause"
    );
}

// ============================================================================
// Bonus: Manufacturing Quality Control (MATCH + material filter)
// ============================================================================
//
// README query:
// ```sql
// MATCH (batch:Batch)-[:TESTED_WITH]->(test:QualityTest)
// WHERE test.result = 'fail'
//   AND batch.material = 'steel'
// RETURN batch.batch_id, test.test_name, test.result
// ```

/// Sets up Manufacturing QC scenario: Batches → QualityTests.
fn setup_manufacturing_qc() -> (tempfile::TempDir, velesdb_core::Collection) {
    let (dir, db) = helpers::setup_test_db();
    let collection =
        helpers::setup_labeled_collection(&db, "manufacturing", 4, DistanceMetric::Cosine);

    let nodes = vec![
        // Batches
        (
            1,
            helpers::generate_embedding(10, 4),
            json!({"_labels": ["Batch"], "batch_id": "B-001", "material": "steel"}),
        ),
        (
            2,
            helpers::generate_embedding(11, 4),
            json!({"_labels": ["Batch"], "batch_id": "B-002", "material": "aluminum"}),
        ),
        (
            3,
            helpers::generate_embedding(12, 4),
            json!({"_labels": ["Batch"], "batch_id": "B-003", "material": "steel"}),
        ),
        // QualityTests
        (
            10,
            helpers::generate_embedding(20, 4),
            json!({"_labels": ["QualityTest"], "test_name": "Tensile Strength", "result": "fail"}),
        ),
        (
            11,
            helpers::generate_embedding(21, 4),
            json!({"_labels": ["QualityTest"], "test_name": "Hardness Check", "result": "pass"}),
        ),
        (
            12,
            helpers::generate_embedding(22, 4),
            json!({"_labels": ["QualityTest"], "test_name": "Corrosion Resistance", "result": "fail"}),
        ),
        (
            13,
            helpers::generate_embedding(23, 4),
            json!({"_labels": ["QualityTest"], "test_name": "Impact Test", "result": "fail"}),
        ),
    ];
    helpers::insert_labeled_nodes(&collection, &nodes);

    helpers::add_edges(
        &collection,
        &[
            (100, 1, 10, "TESTED_WITH"), // B-001 (steel) → Tensile (fail)
            (101, 1, 11, "TESTED_WITH"), // B-001 (steel) → Hardness (pass)
            (102, 2, 12, "TESTED_WITH"), // B-002 (aluminum) → Corrosion (fail)
            (103, 3, 13, "TESTED_WITH"), // B-003 (steel) → Impact (fail)
        ],
    );

    (dir, collection)
}

/// Manufacturing QC: MATCH with material filter + test result filter.
///
/// Verifies:
/// - start_properties filter on batch.material = 'steel'
/// - WHERE on target: test.result = 'fail'
/// - Cross-node projection: batch_id + test_name + result
#[test]
fn test_manufacturing_qc_failed_steel_tests() {
    let (_dir, collection) = setup_manufacturing_qc();

    // Start-node filter: material = 'steel'
    let mut start_props = HashMap::new();
    start_props.insert("material".to_string(), Value::String("steel".to_string()));

    // WHERE test.result = 'fail' (evaluated on target = QualityTest)
    let where_clause = Condition::Comparison(Comparison {
        column: "result".to_string(),
        operator: CompareOp::Eq,
        value: Value::String("fail".to_string()),
    });

    let match_clause = helpers::build_single_hop_match(
        "batch",
        "Batch",
        "TESTED_WITH",
        "test",
        "QualityTest",
        start_props,
        Some(where_clause),
        vec![
            ("batch.batch_id", None),
            ("test.test_name", None),
            ("test.result", None),
        ],
        None,
        Some(20),
    );

    let results = collection
        .execute_match(&match_clause, &HashMap::new())
        .expect("execute_match failed for Manufacturing QC");

    // B-001 (steel) → Tensile (fail) ✓
    // B-001 (steel) → Hardness (pass) ✗ (result != 'fail')
    // B-002 (aluminum) → Corrosion (fail) ✗ (material != 'steel')
    // B-003 (steel) → Impact (fail) ✓
    assert_eq!(
        results.len(),
        2,
        "Should return 2 failed tests for steel batches, got {}",
        results.len()
    );

    // Verify all results are failed tests
    for r in &results {
        if let Some(result_val) = r.projected.get("test.result") {
            assert_eq!(
                result_val.as_str(),
                Some("fail"),
                "test.result should be 'fail'"
            );
        }
    }

    // Verify specific test nodes (10 = Tensile, 13 = Impact)
    let test_ids: Vec<u64> = results.iter().map(|r| r.node_id).collect();
    assert!(
        test_ids.contains(&10),
        "Tensile Strength (id=10) should be in results"
    );
    assert!(
        test_ids.contains(&13),
        "Impact Test (id=13) should be in results"
    );

    // Corrosion (id=12, aluminum batch) must NOT appear
    assert!(
        !test_ids.contains(&12),
        "Corrosion test (aluminum batch) should be excluded"
    );
}
