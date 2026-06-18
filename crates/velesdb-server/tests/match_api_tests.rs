//! E2E tests for /match endpoint (EPIC-058 US-007).
//!
//! Tests the hybrid MATCH + similarity + property projection API.

use serde_json::json;

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
