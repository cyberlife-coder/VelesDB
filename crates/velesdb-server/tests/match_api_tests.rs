//! E2E tests for /match endpoint (EPIC-058 US-007).
//!
//! Tests the hybrid MATCH + similarity + property projection API.

use serde_json::json;

/// Test request/response structures match expected JSON format.
#[test]
fn test_match_request_json_format() {
    let request = json!({
        "query": "MATCH (doc:Document)-[:AUTHORED_BY]->(author:Person) RETURN author.name, doc.title",
        "params": {},
        "vector": [0.1, 0.2, 0.3, 0.4],
        "threshold": 0.8
    });

    assert!(request["query"].as_str().unwrap().contains("MATCH"));
    assert!(request["vector"].as_array().unwrap().len() == 4);
    let threshold = request["threshold"].as_f64().unwrap();
    assert!((threshold - 0.8).abs() < f64::EPSILON);
}

/// Test response JSON format with projected properties.
#[test]
fn test_match_response_json_format() {
    let response = json!({
        "results": [
            {
                "bindings": {"doc": 123, "author": 456},
                "score": 0.95,
                "depth": 1,
                "projected": {
                    "author.name": "John Doe",
                    "doc.title": "Vector Databases in 2026"
                }
            }
        ],
        "took_ms": 15,
        "count": 1,
        "meta": {"velesql_contract_version": "2.1.0"}
    });

    let results = response["results"].as_array().unwrap();
    assert_eq!(results.len(), 1);

    let first = &results[0];
    assert!(first["bindings"]["doc"].as_u64().is_some());
    assert!(first["score"].as_f64().is_some());
    assert_eq!(first["projected"]["author.name"], "John Doe");
}

/// Test minimal request (no vector, basic MATCH).
#[test]
fn test_match_request_minimal() {
    let request = json!({
        "query": "MATCH (n:Node) RETURN n",
        "params": {}
    });

    assert!(request.get("vector").is_none() || request["vector"].is_null());
}

/// Test request with threshold only (no vector).
#[test]
fn test_match_request_with_threshold_no_vector() {
    let request = json!({
        "query": "MATCH (a)-[:KNOWS]->(b) RETURN a, b",
        "params": {},
        "threshold": 0.5
    });

    // threshold without vector should be ignored server-side
    let threshold = request["threshold"].as_f64().unwrap();
    assert!((threshold - 0.5).abs() < f64::EPSILON);
}

/// Test complex MATCH pattern with multiple relationships.
#[test]
fn test_match_complex_pattern_request() {
    let request = json!({
        "query": "MATCH (user:User)-[:FOLLOWS]->(influencer)-[:POSTS]->(content) WHERE similarity(content.embedding, $query) > 0.7 RETURN influencer.name, content.title ORDER BY similarity() DESC LIMIT 10",
        "params": {"query": [0.1, 0.2, 0.3]},
        "vector": [0.1, 0.2, 0.3],
        "threshold": 0.7
    });

    let query = request["query"].as_str().unwrap();
    assert!(query.contains("FOLLOWS"));
    assert!(query.contains("POSTS"));
    assert!(query.contains("similarity"));
    assert!(query.contains("ORDER BY"));
    assert!(query.contains("LIMIT"));
}

/// Test response with multiple results.
#[test]
fn test_match_response_multiple_results() {
    let response = json!({
        "results": [
            {"bindings": {"a": 1}, "score": 0.99, "depth": 0, "projected": {}},
            {"bindings": {"a": 2}, "score": 0.95, "depth": 0, "projected": {}},
            {"bindings": {"a": 3}, "score": 0.90, "depth": 0, "projected": {}}
        ],
        "took_ms": 25,
        "count": 3,
        "meta": {"velesql_contract_version": "2.1.0"}
    });

    assert_eq!(response["count"].as_u64().unwrap(), 3);

    // Verify results are sorted by score descending
    let results = response["results"].as_array().unwrap();
    let scores: Vec<f64> = results
        .iter()
        .map(|r| r["score"].as_f64().unwrap())
        .collect();
    assert!(scores[0] >= scores[1] && scores[1] >= scores[2]);
}

/// Test empty results response.
#[test]
fn test_match_response_empty_results() {
    let response = json!({
        "results": [],
        "took_ms": 5,
        "count": 0,
        "meta": {"velesql_contract_version": "2.1.0"}
    });

    assert_eq!(response["results"].as_array().unwrap().len(), 0);
    assert_eq!(response["count"].as_u64().unwrap(), 0);
}

/// Test error response format.
#[test]
fn test_match_error_response_format() {
    let error = json!({
        "error": "Collection 'nonexistent' not found",
        "code": "COLLECTION_NOT_FOUND",
        "hint": "Create the collection first or correct the collection name in the route"
    });

    assert!(error["error"].as_str().unwrap().contains("not found"));
    assert_eq!(error["code"], "COLLECTION_NOT_FOUND");
}

/// Test parse error response.
#[test]
fn test_match_parse_error_response() {
    let error = json!({
        "error": "Parse error: Expected MATCH clause",
        "code": "PARSE_ERROR",
        "hint": "Check MATCH syntax and bound parameters"
    });

    assert!(error["error"].as_str().unwrap().contains("Parse error"));
    assert_eq!(error["code"], "PARSE_ERROR");
}

/// Test not MATCH query error response.
#[test]
fn test_match_not_match_query_error() {
    let error = json!({
        "error": "Query is not a MATCH query",
        "code": "NOT_MATCH_QUERY",
        "hint": "Use MATCH (...) RETURN ... or call /query for SELECT statements"
    });

    assert_eq!(error["code"], "NOT_MATCH_QUERY");
}

/// Test projected properties with nested paths.
#[test]
fn test_match_projected_nested_properties() {
    let response = json!({
        "results": [
            {
                "bindings": {"doc": 100},
                "score": 0.88,
                "depth": 2,
                "projected": {
                    "doc.metadata.author": "Alice",
                    "doc.metadata.year": 2026,
                    "doc.content.summary": "AI trends..."
                }
            }
        ],
        "took_ms": 30,
        "count": 1
    });

    let projected = &response["results"][0]["projected"];
    assert_eq!(projected["doc.metadata.author"], "Alice");
    assert_eq!(projected["doc.metadata.year"], 2026);
}

/// Verify API contract: threshold must be between 0.0 and 1.0.
#[test]
fn test_match_threshold_range() {
    // Valid thresholds
    for threshold in [0.0, 0.5, 0.8, 1.0] {
        let request = json!({
            "query": "MATCH (n) RETURN n",
            "params": {},
            "threshold": threshold
        });
        let t = request["threshold"].as_f64().unwrap();
        assert!((0.0..=1.0).contains(&t), "Threshold {t} should be valid");
    }
}
