//! Integration tests proving VelesDB's hybrid query value proposition.
//!
//! HYB-01: VelesQL NEAR + scalar filter with ranking identity assertions
//! HYB-02: BM25+cosine hybrid fusion ranking differs from pure vector
//! HYB-03: GraphCollection edges + MATCH traversal returns real results
//!
//! All tests use 4-dimensional orthogonal unit vectors for deterministic ranking.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::uninlined_format_args
)]

use std::collections::HashMap;

use serde_json::json;
use tempfile::TempDir;
use velesdb_core::velesql::Parser;
use velesdb_core::{Database, DistanceMetric, GraphEdge, Point};

/// HYB-01: VelesQL SELECT with NEAR + scalar filter executes against a real corpus
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
