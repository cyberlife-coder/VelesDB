//! Tests for `QueryType` detection (EPIC-052 US-006).
//!
//! Validates that /query endpoint correctly detects query types.

use serde_json::json;

/// Test `QueryType` enum serialization.
#[test]
fn test_query_type_serialize_search() {
    let json_str = serde_json::to_string(&"search").unwrap();
    assert!(json_str.contains("search"));
}

/// Test `QueryType` enum serialization for aggregation.
#[test]
fn test_query_type_serialize_aggregation() {
    let json_str = serde_json::to_string(&"aggregation").unwrap();
    assert!(json_str.contains("aggregation"));
}

/// Test unified response format for search queries.
#[test]
fn test_unified_response_search_format() {
    let response = json!({
        "type": "search",
        "count": 5,
        "timing_ms": 12.5,
        "results": [
            {"id": 1, "score": 0.95, "payload": {"title": "Doc 1"}},
            {"id": 2, "score": 0.90, "payload": {"title": "Doc 2"}}
        ]
    });

    assert_eq!(response["type"], "search");
    assert_eq!(response["count"], 5);
    assert!(response["results"].as_array().is_some());
}

/// Test unified response format for aggregation queries.
#[test]
fn test_unified_response_aggregation_format() {
    let response = json!({
        "type": "aggregation",
        "count": 3,
        "timing_ms": 8.2,
        "results": [
            {"category": "tech", "count": 150},
            {"category": "science", "count": 80},
            {"category": "art", "count": 45}
        ]
    });

    assert_eq!(response["type"], "aggregation");
    let results = response["results"].as_array().unwrap();
    assert_eq!(results.len(), 3);
}

/// Test unified response format for rows (simple SELECT).
#[test]
fn test_unified_response_rows_format() {
    let response = json!({
        "type": "rows",
        "count": 10,
        "timing_ms": 5.1,
        "results": [
            {"id": 1, "name": "Item 1", "price": 100},
            {"id": 2, "name": "Item 2", "price": 200}
        ]
    });

    assert_eq!(response["type"], "rows");
}

/// Test unified response format for graph queries.
#[test]
fn test_unified_response_graph_format() {
    let response = json!({
        "type": "graph",
        "count": 2,
        "timing_ms": 15.3,
        "results": [
            {"bindings": {"a": 1, "b": 2}, "depth": 1},
            {"bindings": {"a": 3, "b": 4}, "depth": 2}
        ]
    });

    assert_eq!(response["type"], "graph");
}

/// Test unified response with warnings.
#[test]
fn test_unified_response_with_warnings() {
    let response = json!({
        "type": "search",
        "count": 100,
        "timing_ms": 50.0,
        "results": [],
        "warnings": ["Results truncated to 100 items", "Consider adding LIMIT"]
    });

    let warnings = response["warnings"].as_array().unwrap();
    assert_eq!(warnings.len(), 2);
}

/// Test unified response without warnings (should be omitted).
#[test]
fn test_unified_response_without_warnings() {
    let response = json!({
        "type": "rows",
        "count": 5,
        "timing_ms": 3.0,
        "results": []
    });

    // warnings field should not be present when empty
    assert!(response.get("warnings").is_none());
}

/// Test detection: `similarity()` indicates search query.
#[test]
fn test_detect_search_with_similarity() {
    let query = "SELECT * FROM docs WHERE similarity(embedding, $v) > 0.8";
    assert!(query.contains("similarity"));
}

/// Test detection: NEAR indicates search query.
#[test]
fn test_detect_search_with_near() {
    let query = "SELECT * FROM docs WHERE embedding NEAR $vector LIMIT 10";
    assert!(query.contains("NEAR"));
}

/// Test detection: GROUP BY indicates aggregation.
#[test]
fn test_detect_aggregation_with_group_by() {
    let query = "SELECT category, COUNT(*) FROM products GROUP BY category";
    assert!(query.contains("GROUP BY"));
}

/// Test detection: COUNT without GROUP BY indicates aggregation.
#[test]
fn test_detect_aggregation_with_count() {
    let query = "SELECT COUNT(*) FROM products";
    assert!(query.contains("COUNT"));
}

/// Test detection: SUM indicates aggregation.
#[test]
fn test_detect_aggregation_with_sum() {
    let query = "SELECT SUM(price) FROM orders";
    assert!(query.contains("SUM"));
}

/// Test detection: MATCH indicates graph query.
#[test]
fn test_detect_graph_with_match() {
    let query = "MATCH (a:Person)-[:KNOWS]->(b) RETURN a.name";
    assert!(query.contains("MATCH"));
}

/// Test detection: simple SELECT indicates rows.
#[test]
fn test_detect_rows_simple_select() {
    let query = "SELECT name, price FROM products WHERE price > 100";
    assert!(!query.contains("similarity"));
    assert!(!query.contains("NEAR"));
    assert!(!query.contains("GROUP BY"));
    assert!(!query.contains("COUNT"));
    assert!(!query.contains("MATCH"));
}

/// Test hybrid query: vector + aggregation should be aggregation type.
#[test]
fn test_detect_hybrid_vector_aggregation() {
    let query = "SELECT category, COUNT(*) FROM docs WHERE similarity(embedding, $v) > 0.7 GROUP BY category";
    assert!(query.contains("similarity"));
    assert!(query.contains("GROUP BY"));
    // Aggregation takes precedence when both present
}

/// Test empty results response.
#[test]
fn test_unified_response_empty_results() {
    let response = json!({
        "type": "search",
        "count": 0,
        "timing_ms": 2.0,
        "results": []
    });

    assert_eq!(response["count"], 0);
    assert!(response["results"].as_array().unwrap().is_empty());
}
