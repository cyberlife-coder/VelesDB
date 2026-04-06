//! Tests for Redis Vector Search connector.

use super::*;
use crate::config::RedisConfig;
use crate::connectors::SourceConnector;
use std::collections::HashMap;

fn test_config() -> RedisConfig {
    RedisConfig {
        url: "redis://localhost:6379".to_string(),
        password: None,
        index: "vectors".to_string(),
        vector_field: "embedding".to_string(),
        key_prefix: "doc:".to_string(),
        payload_fields: vec![],
        filter: None,
    }
}

#[test]
fn test_redis_config_defaults() {
    let json = r#"{"url":"redis://localhost:6379","index":"vectors"}"#;
    let config: RedisConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.vector_field, "embedding");
    assert_eq!(config.key_prefix, "doc:");
    assert!(config.password.is_none());
}

#[test]
fn test_redis_config_with_password() {
    let json = r#"{"url":"redis://localhost:6379","index":"v","password":"secret"}"#;
    let config: RedisConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.password, Some("secret".to_string()));
}

#[test]
fn test_redis_config_with_filter() {
    let json = r#"{"url":"redis://localhost:6379","index":"v","filter":"@status:{active}"}"#;
    let config: RedisConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.filter, Some("@status:{active}".to_string()));
}

#[test]
fn test_redis_connector_new() {
    let connector = RedisConnector::new(test_config());
    assert_eq!(connector.source_type(), "redis");
}

#[test]
fn test_redis_parse_vector_array() {
    let connector = RedisConnector::new(test_config());
    let mut attrs = HashMap::new();
    attrs.insert("embedding".to_string(), serde_json::json!([0.1, 0.2, 0.3]));
    let vector = connector.parse_vector(&attrs).unwrap();
    assert_eq!(vector, vec![0.1, 0.2, 0.3]);
}

#[test]
fn test_redis_parse_vector_string() {
    let connector = RedisConnector::new(test_config());
    let mut attrs = HashMap::new();
    attrs.insert("embedding".to_string(), serde_json::json!("0.1, 0.2, 0.3"));
    let vector = connector.parse_vector(&attrs).unwrap();
    assert_eq!(vector, vec![0.1, 0.2, 0.3]);
}

#[test]
fn test_redis_parse_vector_missing() {
    let connector = RedisConnector::new(test_config());
    let attrs = HashMap::new();
    assert!(connector.parse_vector(&attrs).is_err());
}

#[test]
fn test_redis_extract_id_with_prefix() {
    let connector = RedisConnector::new(test_config());
    assert_eq!(connector.extract_id("doc:123"), "123");
    assert_eq!(connector.extract_id("doc:abc-def"), "abc-def");
}

#[test]
fn test_redis_extract_id_without_prefix() {
    let connector = RedisConnector::new(test_config());
    assert_eq!(connector.extract_id("other:123"), "other:123");
}

#[test]
fn test_redis_extract_payload() {
    let connector = RedisConnector::new(test_config());
    let mut attrs = HashMap::new();
    attrs.insert("embedding".to_string(), serde_json::json!([0.1]));
    attrs.insert("title".to_string(), serde_json::json!("Test"));
    attrs.insert("count".to_string(), serde_json::json!(42));

    let payload = connector.extract_payload(&attrs);
    assert_eq!(payload.len(), 2);
    assert!(!payload.contains_key("embedding"));
    assert!(payload.contains_key("title"));
}

#[test]
fn test_redis_extract_payload_filtered() {
    let mut config = test_config();
    config.payload_fields = vec!["title".to_string()];
    let connector = RedisConnector::new(config);

    let mut attrs = HashMap::new();
    attrs.insert("embedding".to_string(), serde_json::json!([0.1]));
    attrs.insert("title".to_string(), serde_json::json!("T"));
    attrs.insert("count".to_string(), serde_json::json!(42));

    let payload = connector.extract_payload(&attrs);
    assert_eq!(payload.len(), 1);
    assert!(payload.contains_key("title"));
    assert!(!payload.contains_key("count"));
}

// ---------------------------------------------------------------------------
// RESP helper tests
// ---------------------------------------------------------------------------

#[test]
fn test_find_info_int_stride2_correctness() {
    // GIVEN: a flat FT.INFO-style key-value list where a value is "num_docs"
    // (if stride were 1, the value at index 1 would be checked as a key)
    let items = vec![
        redis::Value::BulkString(b"some_key".to_vec()),
        redis::Value::BulkString(b"num_docs".to_vec()), // value at odd index — must NOT match
        redis::Value::BulkString(b"num_docs".to_vec()),  // key at even index — must match
        redis::Value::BulkString(b"42".to_vec()),
    ];
    // WHEN: stride is 2, only the key at index 2 matches, returning 42
    let result = find_info_int(&items, "num_docs");
    assert_eq!(result, Some(42), "stride-1 bug: matched value as key");
}

#[test]
fn test_find_info_int_returns_none_when_missing() {
    let items = vec![
        redis::Value::BulkString(b"index_name".to_vec()),
        redis::Value::BulkString(b"myidx".to_vec()),
    ];
    assert_eq!(find_info_int(&items, "num_docs"), None);
}

#[test]
fn test_decode_vector_blob_le_bytes() {
    // 2 floats: 1.0 (0x3F800000 LE) and -2.5 (0xC0200000 LE)
    let bytes = [0x00u8, 0x00, 0x80, 0x3F, 0x00, 0x00, 0x20, 0xC0];
    let v = decode_vector_blob(&bytes);
    assert_eq!(v.len(), 2);
    assert!(
        (v[0] - 1.0).abs() < 1e-6,
        "v[0] should be 1.0, got {}",
        v[0]
    );
    assert!(
        (v[1] - (-2.5)).abs() < 1e-6,
        "v[1] should be -2.5, got {}",
        v[1]
    );
}

#[test]
fn test_decode_vector_blob_empty() {
    let v = decode_vector_blob(&[]);
    assert!(v.is_empty());
}

#[test]
fn test_parse_ft_search_response_empty() {
    // FT.SEARCH returns [0] when no results match.
    let resp = redis::Value::Array(vec![redis::Value::Int(0)]);
    let points =
        parse_ft_search_response(&resp, "embedding", "doc:").expect("test: empty response");
    assert!(points.is_empty());
}

#[test]
fn test_parse_ft_search_response_single_doc() {
    // Simulate: FT.SEARCH returns 1 document with a JSON-array vector.
    let resp = redis::Value::Array(vec![
        redis::Value::Int(1),
        // Document key
        redis::Value::BulkString(b"doc:42".to_vec()),
        // Field-value pairs
        redis::Value::Array(vec![
            redis::Value::BulkString(b"embedding".to_vec()),
            redis::Value::BulkString(b"[0.1, 0.2, 0.3]".to_vec()),
            redis::Value::BulkString(b"title".to_vec()),
            redis::Value::BulkString(b"\"Hello World\"".to_vec()),
        ]),
    ]);

    let points =
        parse_ft_search_response(&resp, "embedding", "doc:").expect("test: single doc parse");

    assert_eq!(points.len(), 1);
    assert_eq!(points[0].id, "42");
    assert_eq!(points[0].vector, vec![0.1, 0.2, 0.3]);
    assert!(points[0].payload.contains_key("title"));
    assert!(!points[0].payload.contains_key("embedding"));
}

#[test]
fn test_parse_ft_search_response_multiple_docs() {
    let resp = redis::Value::Array(vec![
        redis::Value::Int(2),
        redis::Value::BulkString(b"doc:1".to_vec()),
        redis::Value::Array(vec![
            redis::Value::BulkString(b"embedding".to_vec()),
            redis::Value::BulkString(b"[1.0, 2.0]".to_vec()),
        ]),
        redis::Value::BulkString(b"doc:2".to_vec()),
        redis::Value::Array(vec![
            redis::Value::BulkString(b"embedding".to_vec()),
            redis::Value::BulkString(b"[3.0, 4.0]".to_vec()),
        ]),
    ]);

    let points =
        parse_ft_search_response(&resp, "embedding", "doc:").expect("test: multiple docs parse");

    assert_eq!(points.len(), 2);
    assert_eq!(points[0].id, "1");
    assert_eq!(points[0].vector, vec![1.0, 2.0]);
    assert_eq!(points[1].id, "2");
    assert_eq!(points[1].vector, vec![3.0, 4.0]);
}

#[test]
fn test_parse_ft_search_response_no_prefix_match() {
    // Document key doesn't start with the configured prefix.
    let resp = redis::Value::Array(vec![
        redis::Value::Int(1),
        redis::Value::BulkString(b"other:99".to_vec()),
        redis::Value::Array(vec![
            redis::Value::BulkString(b"embedding".to_vec()),
            redis::Value::BulkString(b"[5.0]".to_vec()),
        ]),
    ]);

    let points =
        parse_ft_search_response(&resp, "embedding", "doc:").expect("test: no prefix match");

    assert_eq!(points.len(), 1);
    // Full key is used as ID when prefix doesn't match.
    assert_eq!(points[0].id, "other:99");
}

#[test]
fn test_resp_value_to_json_types() {
    // BulkString containing a number string parses to JSON number.
    let val = resp_value_to_json(&redis::Value::BulkString(b"42".to_vec()));
    assert_eq!(val, serde_json::json!(42));

    // BulkString containing a plain string stays as string.
    let val = resp_value_to_json(&redis::Value::BulkString(b"hello".to_vec()));
    assert_eq!(val, serde_json::json!("hello"));

    // Int maps to JSON number.
    let val = resp_value_to_json(&redis::Value::Int(7));
    assert_eq!(val, serde_json::json!(7));

    // Nil maps to JSON null.
    let val = resp_value_to_json(&redis::Value::Nil);
    assert!(val.is_null());
}

#[test]
fn test_build_ft_search_cmd_no_payload_fields() {
    // When payload_fields is empty, no RETURN clause is added.
    let cmd = build_ft_search_cmd("myidx", "*", 0, 10, "embedding", &[]);
    let packed = cmd.get_packed_command();
    let cmd_str = String::from_utf8_lossy(&packed);
    // Should contain FT.SEARCH, index, query, LIMIT but no RETURN.
    assert!(cmd_str.contains("FT.SEARCH"));
    assert!(cmd_str.contains("myidx"));
    assert!(!cmd_str.contains("RETURN"));
}

#[test]
fn test_build_ft_search_cmd_with_payload_fields() {
    let fields = vec!["title".to_string(), "score".to_string()];
    let cmd = build_ft_search_cmd("myidx", "*", 5, 20, "embedding", &fields);
    let packed = cmd.get_packed_command();
    let cmd_str = String::from_utf8_lossy(&packed);
    assert!(cmd_str.contains("RETURN"));
    assert!(cmd_str.contains("title"));
    assert!(cmd_str.contains("score"));
    // Vector field should be auto-added.
    assert!(cmd_str.contains("embedding"));
}

#[test]
fn test_parse_ft_search_response_binary_blob_vector() {
    // GIVEN: a FT.SEARCH response where the vector field is a raw LE f32 blob
    // (this is the default RediSearch storage format for VECTOR fields).
    let mut blob = Vec::new();
    blob.extend_from_slice(&f32::to_le_bytes(1.0_f32));
    blob.extend_from_slice(&f32::to_le_bytes(2.5_f32));

    let resp = redis::Value::Array(vec![
        redis::Value::Int(1),
        redis::Value::BulkString(b"doc:7".to_vec()),
        redis::Value::Array(vec![
            redis::Value::BulkString(b"embedding".to_vec()),
            redis::Value::BulkString(blob),
            redis::Value::BulkString(b"title".to_vec()),
            redis::Value::BulkString(b"Hello".to_vec()),
        ]),
    ]);

    // WHEN: parsing the response
    let points =
        parse_ft_search_response(&resp, "embedding", "doc:").expect("test: binary blob");

    // THEN: vector is correctly decoded from LE f32 bytes
    assert_eq!(points.len(), 1);
    assert_eq!(points[0].id, "7");
    assert_eq!(points[0].vector.len(), 2);
    assert!(
        (points[0].vector[0] - 1.0).abs() < 1e-6,
        "expected 1.0, got {}",
        points[0].vector[0]
    );
    assert!(
        (points[0].vector[1] - 2.5).abs() < 1e-6,
        "expected 2.5, got {}",
        points[0].vector[1]
    );
    assert!(points[0].payload.contains_key("title"));
}

#[tokio::test]
async fn test_extract_batch_fails_when_not_connected() {
    // GIVEN: a connector that has not been connected
    let connector = RedisConnector::new(test_config());

    // WHEN: extract_batch is called without connect()
    let result = connector.extract_batch(None, 10).await;

    // THEN: an error is returned indicating not connected
    assert!(result.is_err(), "extract_batch should fail without connect()");
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("Not connected"),
        "expected 'Not connected' in error, got: {err}"
    );
}

