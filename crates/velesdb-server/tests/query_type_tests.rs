//! Tests for `QueryType` detection (EPIC-052 US-006).
//!
//! Validates that /query endpoint correctly detects query types.

use serde_json::json;
use velesdb_core::api_types::{QueryType, UnifiedQueryResponse};

/// Test unified response format for rows (simple SELECT).
///
/// Guards `#[serde(rename = "type")]` + `rename_all="lowercase"` on
/// `QueryType::Rows` and `skip_serializing_if = "Vec::is_empty"` on warnings.
#[test]
fn test_unified_response_rows_format() {
    let resp = UnifiedQueryResponse {
        query_type: QueryType::Rows,
        count: 2,
        timing_ms: 5.1,
        results: json!([
            {"id": 1, "name": "Item 1", "price": 100},
            {"id": 2, "name": "Item 2", "price": 200}
        ]),
        warnings: vec![],
    };
    let v = serde_json::to_value(&resp).unwrap();
    // Guards #[serde(rename = "type")] + rename_all="lowercase" on QueryType::Rows.
    assert_eq!(v["type"], "rows");
    // Guards skip_serializing_if = "Vec::is_empty" on warnings.
    assert!(v.get("warnings").is_none());
    assert_eq!(v["count"], 2);
}
