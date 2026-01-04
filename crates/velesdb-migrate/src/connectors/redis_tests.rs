//! Tests for Redis Vector Search connector.

use super::*;
use crate::connectors::SourceConnector;

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
fn test_redis_build_api_url() {
    assert_eq!(
        RedisConnector::build_api_url("redis://localhost:6379"),
        "http://localhost:6379"
    );
    assert_eq!(
        RedisConnector::build_api_url("rediss://cloud.redis.io:6380/"),
        "https://cloud.redis.io:6380"
    );
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
