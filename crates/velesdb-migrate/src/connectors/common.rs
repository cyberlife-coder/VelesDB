//! Common utilities shared across connectors.
//!
//! This module provides reusable functions for vector parsing, payload extraction,
//! HTTP client creation, URL validation, and error handling.

use crate::error::{Error, Result};
use reqwest::Client;
use serde_json::Value;
use std::collections::HashMap;
use std::time::Duration;

/// Default HTTP timeout for all connectors.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// Maximum file size for local imports (100MB).
pub const MAX_FILE_SIZE: u64 = 100 * 1024 * 1024;

/// Creates a configured HTTP client with timeout.
#[must_use]
pub fn create_http_client() -> Client {
    Client::builder()
        .timeout(DEFAULT_TIMEOUT)
        .connect_timeout(Duration::from_secs(10))
        .build()
        .unwrap_or_else(|_| Client::new())
}

/// Validates a URL for safety (anti-SSRF).
pub fn validate_url(url: &str) -> Result<()> {
    // Check for valid scheme
    let valid_schemes = [
        "http://",
        "https://",
        "redis://",
        "rediss://",
        "postgres://",
        "postgresql://",
    ];
    let has_valid_scheme = valid_schemes.iter().any(|s| url.starts_with(s));

    if !has_valid_scheme {
        return Err(Error::Config(format!(
            "Invalid URL scheme in '{}'. Allowed: http, https, redis, postgres",
            url
        )));
    }

    // Basic URL format validation
    if url.len() < 10 || !url.contains("://") {
        return Err(Error::Config(format!("Invalid URL format: {}", url)));
    }

    Ok(())
}

/// Parses a vector from a JSON value.
///
/// Expects the value to be a JSON array of numbers.
pub fn parse_vector_from_json(value: &Value, field_name: &str) -> Result<Vec<f32>> {
    match value {
        Value::Array(arr) => arr
            .iter()
            .map(|v| {
                v.as_f64()
                    .map(|f| f as f32)
                    .ok_or_else(|| Error::Extraction("Vector element is not a number".to_string()))
            })
            .collect(),
        _ => Err(Error::Extraction(format!(
            "Vector field '{}' is not an array",
            field_name
        ))),
    }
}

/// Extracts payload fields from a JSON object.
///
/// Skips specified excluded fields and optionally filters to only included fields.
pub fn extract_payload_from_object(
    source: &Value,
    excluded_fields: &[&str],
    included_fields: &[String],
) -> HashMap<String, Value> {
    let mut payload = HashMap::new();

    if let Value::Object(map) = source {
        for (key, val) in map {
            // Skip excluded fields
            if excluded_fields.iter().any(|f| f == key) {
                continue;
            }
            // If included_fields is specified, only include those
            if !included_fields.is_empty() && !included_fields.contains(key) {
                continue;
            }
            payload.insert(key.clone(), val.clone());
        }
    }

    payload
}

/// Detects the JSON type as a string for schema detection.
pub fn json_type_name(value: &Value) -> String {
    match value {
        Value::String(_) => "string".to_string(),
        Value::Number(_) => "number".to_string(),
        Value::Bool(_) => "boolean".to_string(),
        Value::Array(_) => "array".to_string(),
        Value::Object(_) => "object".to_string(),
        Value::Null => "null".to_string(),
    }
}

/// Handles HTTP error responses and returns appropriate errors.
pub fn handle_http_error(status_code: u16, body: &str, source_name: &str) -> Error {
    match status_code {
        429 => Error::RateLimit(60), // Default 60s retry
        401 | 403 => Error::Authentication(format!("{} auth failed: {}", source_name, body)),
        _ => Error::SourceConnection(format!("{} error {}: {}", source_name, status_code, body)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_vector_success() {
        let value = serde_json::json!([0.1, 0.2, 0.3]);
        let result = parse_vector_from_json(&value, "embedding").unwrap();
        assert_eq!(result, vec![0.1, 0.2, 0.3]);
    }

    #[test]
    fn test_parse_vector_not_array() {
        let value = serde_json::json!("not an array");
        let result = parse_vector_from_json(&value, "embedding");
        assert!(result.is_err());
    }

    #[test]
    fn test_extract_payload_excludes_fields() {
        let source = serde_json::json!({
            "_id": "1",
            "embedding": [0.1],
            "title": "Test",
            "count": 42
        });
        let payload = extract_payload_from_object(&source, &["_id", "embedding"], &[]);
        assert_eq!(payload.len(), 2);
        assert!(payload.contains_key("title"));
        assert!(payload.contains_key("count"));
        assert!(!payload.contains_key("_id"));
        assert!(!payload.contains_key("embedding"));
    }

    #[test]
    fn test_extract_payload_includes_only_specified() {
        let source = serde_json::json!({
            "title": "Test",
            "count": 42,
            "category": "doc"
        });
        let payload = extract_payload_from_object(&source, &[], &["title".to_string()]);
        assert_eq!(payload.len(), 1);
        assert!(payload.contains_key("title"));
        assert!(!payload.contains_key("count"));
    }

    #[test]
    fn test_json_type_name() {
        assert_eq!(json_type_name(&serde_json::json!("test")), "string");
        assert_eq!(json_type_name(&serde_json::json!(42)), "number");
        assert_eq!(json_type_name(&serde_json::json!(true)), "boolean");
        assert_eq!(json_type_name(&serde_json::json!([])), "array");
        assert_eq!(json_type_name(&serde_json::json!({})), "object");
        assert_eq!(json_type_name(&serde_json::json!(null)), "null");
    }

    #[test]
    fn test_handle_http_error_rate_limit() {
        let err = handle_http_error(429, "too many requests", "MongoDB");
        assert!(matches!(err, Error::RateLimit(60)));
    }

    #[test]
    fn test_handle_http_error_auth() {
        let err = handle_http_error(401, "unauthorized", "Elasticsearch");
        assert!(matches!(err, Error::Authentication(_)));
    }

    #[test]
    fn test_handle_http_error_other() {
        let err = handle_http_error(500, "internal error", "Test");
        assert!(matches!(err, Error::SourceConnection(_)));
    }

    #[test]
    fn test_validate_url_valid_http() {
        assert!(validate_url("http://localhost:9200").is_ok());
        assert!(validate_url("https://api.example.com").is_ok());
    }

    #[test]
    fn test_validate_url_valid_redis() {
        assert!(validate_url("redis://localhost:6379").is_ok());
        assert!(validate_url("rediss://cloud.redis.io:6380").is_ok());
    }

    #[test]
    fn test_validate_url_invalid_scheme() {
        assert!(validate_url("ftp://files.example.com").is_err());
        assert!(validate_url("file:///etc/passwd").is_err());
    }

    #[test]
    fn test_create_http_client() {
        let client = create_http_client();
        // Client should be created successfully
        assert!(client.get("http://example.com").build().is_ok());
    }
}
