#![cfg(feature = "persistence")]
#![allow(deprecated)] // Tests use legacy Collection.
//! Integration tests proving `VelesDB`'s hybrid query value proposition.
//!
//! HYB-01: `VelesQL` NEAR + scalar filter with ranking identity assertions
//! HYB-02: BM25+cosine hybrid fusion ranking differs from pure vector
//! HYB-03: `GraphCollection` edges + MATCH traversal returns real results
//!
//! All tests use 4-dimensional orthogonal unit vectors for deterministic ranking.

use std::collections::HashMap;

use serde_json::json;
use tempfile::TempDir;
use velesdb_core::velesql::Parser;
use velesdb_core::{Database, DistanceMetric, GraphEdge, Point};

/// HYB-01: `VelesQL` SELECT with NEAR + scalar filter executes against a real corpus
/// and returns only docs matching the filter, with the highest-similarity doc ranked first.
#[test]
fn test_hyb01_velesql_near_scalar_filter_ranking() {
    let dir = TempDir::new().expect("tempdir");
    let db = Database::open(dir.path()).expect("open db");
    db.create_collection("corpus", 4, DistanceMetric::Cosine)
        .expect("create collection");
    let collection = db.get_collection("corpus").expect("get collection");

    // 4-dimensional corpus: orthogonal-ish vectors with category payloads
    collection
        .upsert(vec![
            Point::new(
                1,
                vec![1.0, 0.0, 0.0, 0.0],
                Some(json!({"category": "tech"})),
            ),
            Point::new(
                2,
                vec![0.9, 0.1, 0.0, 0.0],
                Some(json!({"category": "tech"})),
            ),
            Point::new(
                3,
                vec![0.5, 0.5, 0.0, 0.0],
                Some(json!({"category": "other"})),
            ),
            Point::new(
                4,
                vec![0.0, 1.0, 0.0, 0.0],
                Some(json!({"category": "tech"})),
            ),
        ])
        .expect("upsert points");

    let query_str =
        "SELECT * FROM corpus WHERE vector NEAR $v AND category = 'tech' ORDER BY similarity(vector, $v) DESC LIMIT 5";
    let query = Parser::parse(query_str).expect("parse VelesQL query");

    let mut params = HashMap::new();
    params.insert("v".to_string(), json!([1.0_f32, 0.0, 0.0, 0.0]));

    let results = collection
        .execute_query(&query, &params)
        .expect("execute query");

    // 1. Non-empty
    assert!(
        !results.is_empty(),
        "NEAR query with scalar filter must return results"
    );

    // 2. All results have category='tech' (doc id=3 with category='other' is filtered out)
    for r in &results {
        let cat = r
            .point
            .payload
            .as_ref()
            .and_then(|p| p.get("category"))
            .and_then(|v| v.as_str());
        assert_eq!(
            cat,
            Some("tech"),
            "All results must have category='tech', got {:?} for point id={}",
            cat,
            r.point.id
        );
    }

    // 3. Exact match vector [1,0,0,0] must rank first under cosine
    assert_eq!(
        results[0].point.id, 1,
        "Point id=1 (exact match [1,0,0,0]) must rank first, got id={}",
        results[0].point.id
    );

    // 4. Decreasing score order
    for i in 0..results.len().saturating_sub(1) {
        assert!(
            results[i].score >= results[i + 1].score,
            "Results must be in decreasing score order: results[{}].score={} < results[{}].score={}",
            i,
            results[i].score,
            i + 1,
            results[i + 1].score
        );
    }
}

/// HYB-02: A `hybrid_search()` call on a corpus where vector and BM25 signals diverge
/// produces a ranking that differs from pure-vector ranking.
#[test]
fn test_hyb02_fusion_ranking_differs_from_pure_vector() {
    let dir = TempDir::new().expect("tempdir");
    let db = Database::open(dir.path()).expect("open db");
    db.create_collection("docs", 4, DistanceMetric::Cosine)
        .expect("create collection");
    let collection = db.get_collection("docs").expect("get collection");

    // 3-doc corpus: vector and BM25 signals intentionally diverge
    collection
        .upsert(vec![
            Point::new(
                1,
                vec![1.0, 0.0, 0.0, 0.0],
                Some(json!({"text": "chocolate cake baking"})),
            ),
            Point::new(
                2,
                vec![0.0, 1.0, 0.0, 0.0],
                Some(json!({"text": "rust programming systems"})),
            ),
            Point::new(
                3,
                vec![0.7, 0.3, 0.0, 0.0],
                Some(json!({"text": "rust performance fast"})),
            ),
        ])
        .expect("upsert points");

    // Vector-only: id=1 should win (exact match to query [1,0,0,0])
    let vector_results = collection
        .search(&[1.0, 0.0, 0.0, 0.0_f32], 3)
        .expect("vector search");
    assert_eq!(
        vector_results[0].point.id, 1,
        "Vector-only: id=1 (exact match) must rank first"
    );

    // Text-only: id=1 cannot win (no "rust" in "chocolate cake baking")
    let text_results = collection.text_search("rust", 3).unwrap();
    assert!(
        !text_results.is_empty(),
        "BM25 must index payload fields -- check auto-indexing or use 'text' field instead of 'content'"
    );
    assert_ne!(
        text_results[0].point.id, 1,
        "Text-only: id=1 has no 'rust' and must not rank first"
    );

    // Hybrid fusion: must differ from pure vector ranking
    let hybrid_results = collection
        .hybrid_search(&[1.0, 0.0, 0.0, 0.0_f32], "rust", 3, Some(0.5))
        .expect("hybrid search");

    let vector_ids: Vec<u64> = vector_results.iter().map(|r| r.point.id).collect();
    let hybrid_ids: Vec<u64> = hybrid_results.iter().map(|r| r.point.id).collect();

    assert_ne!(
        hybrid_ids, vector_ids,
        "Fusion ranking must differ from pure vector when BM25 signal diverges"
    );
}

/// HYB-03: A `VelesQL` MATCH traversal over real `GraphCollection` edges returns results
/// (traversal executed, not just parsed).
#[test]
fn test_hyb03_graph_match_traversal_returns_real_edges() {
    let dir = TempDir::new().expect("tempdir");
    let db = Database::open(dir.path()).expect("open db");
    db.create_collection("kg", 4, DistanceMetric::Cosine)
        .expect("create collection");
    let collection = db.get_collection("kg").expect("get collection");

    // 3-node knowledge graph with _labels for MATCH label filtering
    collection
        .upsert(vec![
            Point::new(
                1,
                vec![1.0, 0.0, 0.0, 0.0],
                Some(json!({"_labels": ["Author"], "name": "Alice"})),
            ),
            Point::new(
                2,
                vec![0.0, 1.0, 0.0, 0.0],
                Some(json!({"_labels": ["Document"], "title": "Rust Guide"})),
            ),
            Point::new(
                3,
                vec![0.0, 0.0, 1.0, 0.0],
                Some(json!({"_labels": ["Document"], "title": "Python Guide"})),
            ),
        ])
        .expect("upsert points");

    // Add real edges: Document->Author
    let edge1 = GraphEdge::new(1, 2, 1, "AUTHORED_BY").expect("create edge 1");
    let edge2 = GraphEdge::new(2, 3, 1, "AUTHORED_BY").expect("create edge 2");
    collection.add_edge(edge1).expect("add edge 1");
    collection.add_edge(edge2).expect("add edge 2");

    // MATCH traversal query
    let query = Parser::parse("MATCH (d:Document)-[:AUTHORED_BY]->(a:Author) RETURN d, a LIMIT 10")
        .expect("parse MATCH query");

    let results = collection
        .execute_query(&query, &HashMap::new())
        .expect("execute MATCH query");

    // The MATCH pattern traverses Document->Author edges.  Both edges (2->1
    // and 3->1) should produce a matched path, so we expect exactly 2 results.
    // The returned point corresponds to the Author node (id=1) in both paths.
    let result_ids: Vec<u64> = results.iter().map(|r| r.point.id).collect();
    assert_eq!(
        result_ids.len(),
        2,
        "Both Document->Author edges must be traversed, got {} results: {result_ids:?}",
        result_ids.len(),
    );
    assert!(
        result_ids.iter().all(|&id| id == 1),
        "All traversal results should reach Author node (id=1), got {result_ids:?}",
    );
}
