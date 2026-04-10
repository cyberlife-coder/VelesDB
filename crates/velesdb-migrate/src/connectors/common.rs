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
        .unwrap_or_else(|err| {
            tracing::warn!(
                error = %err,
                "Failed to build HTTP client with timeouts; falling back to default (unbounded timeout)"
            );
            Client::new()
        })
}

/// URL schemes accepted by migration connectors.
const ALLOWED_SCHEMES: &[&str] = &["http", "https", "redis", "rediss", "postgres", "postgresql"];

/// Validates a URL for use as a migration source or sink endpoint.
///
/// Applies anti-SSRF checks aligned with OWASP guidance:
/// 1. Scheme must belong to [`ALLOWED_SCHEMES`].
/// 2. URL userinfo (`user:pass@host`) is rejected; credentials must be
///    supplied via the connector's explicit authentication fields.
/// 3. Host component must be present and non-empty.
/// 4. Host must not resolve to a loopback, private (RFC 1918 / ULA),
///    link-local, or cloud-metadata address.
/// 5. Domain names ending in `.localhost`, `.local`, `.internal`, or
///    `.arpa`, or the bare label `localhost`, are rejected.
///
/// # Local development escape hatch
///
/// The environment variable `VELESDB_MIGRATE_ALLOW_PRIVATE_NETWORKS=1`
/// disables checks (4) and (5) to support local docker-compose stacks.
/// Checks (1)–(3) always apply. This variable must not be set in
/// production deployments.
///
/// # Errors
///
/// Returns [`Error::Config`] with a message that includes the rejected
/// input and the specific rule that failed.
pub fn validate_url(input: &str) -> Result<()> {
    // Delegate RFC 3986 parsing to the `url` crate.
    let parsed =
        url::Url::parse(input).map_err(|e| Error::Config(format!("Invalid URL '{input}': {e}")))?;

    // (1) Scheme allowlist.
    let scheme = parsed.scheme();
    if !ALLOWED_SCHEMES.contains(&scheme) {
        return Err(Error::Config(format!(
            "Disallowed URL scheme '{scheme}' in '{input}'. \
             Allowed: {}",
            ALLOWED_SCHEMES.join(", ")
        )));
    }

    // (2) Reject embedded userinfo to prevent credential smuggling and
    //     parser-confusion attacks where a crafted `user@host` component
    //     overrides the caller's intended target.
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(Error::Config(format!(
            "URL '{input}' must not contain userinfo (user:pass@host). \
             Pass credentials via the connector's explicit auth config."
        )));
    }

    // (3) Host presence and non-emptiness.
    let host = parsed
        .host()
        .ok_or_else(|| Error::Config(format!("URL '{input}' is missing a host component")))?;
    if let url::Host::Domain(d) = &host {
        if d.is_empty() {
            return Err(Error::Config(format!(
                "URL '{input}' has an empty host component"
            )));
        }
    }

    // Local development escape hatch: bypass checks (4) and (5) only.
    let allow_private = std::env::var("VELESDB_MIGRATE_ALLOW_PRIVATE_NETWORKS")
        .ok()
        .filter(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .is_some();
    if allow_private {
        return Ok(());
    }

    // (4) and (5) Private-range and reserved-hostname rejection.
    reject_unsafe_host(&host, input)
}

/// Rejects hosts that resolve to private, loopback, link-local, or
/// cloud-metadata endpoints. Implements checks (4) and (5) documented on
/// [`validate_url`].
///
/// Returns `Ok(())` if the host is publicly routable, `Err(Error::Config)`
/// otherwise.
fn reject_unsafe_host(host: &url::Host<&str>, input: &str) -> Result<()> {
    match host {
        url::Host::Ipv4(ip) => {
            if ip.is_loopback() || ip.is_private() || ip.is_link_local() {
                return Err(Error::Config(format!(
                    "URL '{input}' targets a non-public IPv4 range ({ip}): \
                     loopback, RFC 1918 private, or link-local. \
                     Set VELESDB_MIGRATE_ALLOW_PRIVATE_NETWORKS=1 for \
                     local development."
                )));
            }
            // `is_link_local()` already covers 169.254.0.0/16; the
            // additional exact-match check produces a clearer diagnostic
            // for the well-known cloud metadata endpoint.
            if ip.octets() == [169, 254, 169, 254] {
                return Err(Error::Config(format!(
                    "URL '{input}' targets the cloud metadata endpoint \
                     (169.254.169.254)"
                )));
            }
        }
        url::Host::Ipv6(ip) => {
            if ip.is_loopback() || ip.is_unspecified() {
                return Err(Error::Config(format!(
                    "URL '{input}' targets an IPv6 loopback or unspecified \
                     address ({ip})"
                )));
            }
            // Detect fe80::/10 (link-local) and fc00::/7 (unique local)
            // via their IPv6 prefix masks; `std::net::Ipv6Addr` lacks
            // stable is_unique_local / is_unicast_link_local on MSRV 1.83.
            let first = ip.segments()[0];
            if (first & 0xffc0) == 0xfe80 {
                return Err(Error::Config(format!(
                    "URL '{input}' targets an IPv6 link-local address ({ip})"
                )));
            }
            if (first & 0xfe00) == 0xfc00 {
                return Err(Error::Config(format!(
                    "URL '{input}' targets an IPv6 unique-local address ({ip})"
                )));
            }
        }
        url::Host::Domain(name) => {
            let lower = name.to_ascii_lowercase();
            // Reject reserved suffixes that name on-host or internal-only
            // services. These cover both standards-reserved labels
            // (RFC 6761 `.localhost`, RFC 8375 `.home.arpa`) and common
            // private-DNS conventions (`.internal`, `.local`).
            const RESERVED_SUFFIXES: &[&str] =
                &["localhost", ".localhost", ".local", ".internal", ".arpa"];
            let is_reserved = lower == "localhost"
                || RESERVED_SUFFIXES
                    .iter()
                    .any(|s| lower == s.trim_start_matches('.') || lower.ends_with(s));
            if is_reserved {
                return Err(Error::Config(format!(
                    "URL '{input}' targets reserved hostname '{lower}' \
                     (localhost / .local / .internal / .arpa). \
                     Set VELESDB_MIGRATE_ALLOW_PRIVATE_NETWORKS=1 for \
                     local development."
                )));
            }
        }
    }
    Ok(())
}

/// Returns `true` if the sparse vector is non-empty and has matching indices/values lengths.
#[must_use]
pub fn is_valid_sparse_vector(indices: &[u32], values: &[f32]) -> bool {
    !indices.is_empty() && indices.len() == values.len()
}

/// Parses a vector from a JSON value.
///
/// Expects the value to be a JSON array of numbers.
// Reason: JSON numbers parsed as f64; f32 truncation is expected for embeddings.
#[allow(clippy::cast_possible_truncation)]
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
#[must_use]
pub fn json_type_name(value: &Value) -> &'static str {
    match value {
        Value::String(_) => "string",
        Value::Number(_) => "number",
        Value::Bool(_) => "boolean",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
        Value::Null => "null",
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

/// Returns a cached schema or an error indicating the connector is not connected.
///
/// Use this in `get_schema()` implementations for connectors that populate
/// `self.schema` during `connect()`.
pub fn cached_schema(
    schema: &Option<crate::connectors::SourceSchema>,
) -> Result<crate::connectors::SourceSchema> {
    schema
        .clone()
        .ok_or_else(|| Error::SourceConnection("Not connected".to_string()))
}

/// Extracts a string ID from a JSON value.
///
/// Handles numeric IDs (converted to string) and string IDs.
/// Falls back to a new UUID v4 if the value is missing or has an unexpected type.
pub fn extract_id_from_value(value: Option<Value>) -> String {
    value
        .and_then(|v| match v {
            Value::Number(n) => Some(n.to_string()),
            Value::String(s) => Some(s),
            _ => None,
        })
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string())
}

/// Formats an optional count for display, returning "unknown" when absent.
pub fn format_count(count: Option<u64>) -> String {
    count.map_or_else(|| "unknown".to_string(), |c| c.to_string())
}

/// Detects payload fields from a sample JSON document, excluding specified fields.
///
/// Each non-excluded key produces a `FieldInfo` with type inferred via [`json_type_name`].
pub fn detect_fields_from_sample(
    source: &Value,
    excluded_fields: &[&str],
) -> Vec<crate::connectors::FieldInfo> {
    let Value::Object(map) = source else {
        return Vec::new();
    };
    map.iter()
        .filter(|(key, _)| !excluded_fields.iter().any(|f| f == key))
        .map(|(key, val)| crate::connectors::FieldInfo {
            name: key.clone(),
            field_type: json_type_name(val).to_string(),
            indexed: false,
        })
        .collect()
}

/// Checks an HTTP response status and returns an error on failure.
///
/// This eliminates the repeated pattern of:
/// ```text
/// if !resp.status().is_success() {
///     let status = resp.status();
///     let body = resp.text().await.unwrap_or_default();
///     return Err(Error::...(format!("... {status} - {body}")));
/// }
/// ```
///
/// The error message always includes the status code and body text so that
/// downstream retry logic (which pattern-matches on "429", "500", etc.) works
/// identically to the hand-written checks it replaces.
///
/// # Errors
///
/// Returns `Error::SourceConnection` with a message containing the HTTP status
/// and response body when the response is not 2xx.
pub async fn check_response(
    response: reqwest::Response,
    source_name: &str,
    operation: &str,
) -> Result<reqwest::Response> {
    if response.status().is_success() {
        return Ok(response);
    }
    let status = response.status().as_u16();
    let body = response.text().await.unwrap_or_default();
    Err(handle_http_error(
        status,
        &format!("{operation} failed: {body}"),
        source_name,
    ))
}

/// Builds an [`ExtractedBatch`] from collected points using numeric offset pagination.
///
/// Computes `has_more` by comparing `points.len()` to `batch_size`, and
/// produces `next_offset = current + points.len()` when there are more results.
///
/// Use this for connectors that paginate with a simple numeric skip/offset
/// (MongoDB, Redis, Elasticsearch scroll_after excluded, Supabase, Milvus, ChromaDB).
pub fn build_numeric_offset_batch(
    points: Vec<crate::connectors::ExtractedPoint>,
    batch_size: usize,
    current_offset: u64,
) -> crate::connectors::ExtractedBatch {
    let has_more = points.len() == batch_size;
    let next_offset = if has_more {
        Some(serde_json::json!(current_offset + points.len() as u64))
    } else {
        None
    };
    crate::connectors::ExtractedBatch {
        points,
        next_offset,
        has_more,
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

    // ---- validate_url: SSRF regression test suite ----
    //
    // Each rule documented on `validate_url` is covered by at least one
    // positive and one negative test. The suite tracks the rule numbers
    // from the validate_url docstring for traceability during audits.

    #[test]
    fn test_validate_url_allows_public_https() {
        assert!(validate_url("https://api.openai.com").is_ok());
        assert!(validate_url("https://example.com:443/path?query=1").is_ok());
    }

    #[test]
    fn test_validate_url_allows_public_http() {
        assert!(validate_url("http://pinecone.io").is_ok());
    }

    #[test]
    fn test_validate_url_rejects_ftp_scheme() {
        let err = validate_url("ftp://files.example.com").unwrap_err();
        assert!(err.to_string().contains("Disallowed URL scheme 'ftp'"));
    }

    #[test]
    fn test_validate_url_rejects_file_scheme() {
        assert!(validate_url("file:///etc/passwd").is_err());
    }

    #[test]
    fn test_validate_url_rejects_gopher_scheme() {
        assert!(validate_url("gopher://example.com:70").is_err());
    }

    #[test]
    fn test_validate_url_rejects_userinfo_in_url() {
        // Regression: `http://alt-host@victim.com` can be misinterpreted
        // by naive URL parsers as fetching `alt-host`. Reject any URL
        // that embeds credentials regardless of the host component.
        let err = validate_url("http://alt-host@victim.com").unwrap_err();
        assert!(err.to_string().contains("must not contain userinfo"));
    }

    #[test]
    fn test_validate_url_rejects_password_in_url() {
        assert!(validate_url("https://user:pass@example.com").is_err());
    }

    #[test]
    fn test_validate_url_rejects_aws_metadata_ipv4() {
        let err = validate_url("http://169.254.169.254/latest/meta-data/").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("169.254.169.254") || msg.contains("link-local"));
    }

    #[test]
    fn test_validate_url_rejects_loopback_ipv4() {
        assert!(validate_url("http://127.0.0.1:8080").is_err());
        assert!(validate_url("http://127.0.0.1").is_err());
    }

    #[test]
    fn test_validate_url_rejects_rfc1918_10() {
        assert!(validate_url("http://10.0.0.1").is_err());
        assert!(validate_url("http://10.10.10.10:9200").is_err());
    }

    #[test]
    fn test_validate_url_rejects_rfc1918_172() {
        assert!(validate_url("http://172.16.0.1").is_err());
        assert!(validate_url("http://172.31.255.255").is_err());
    }

    #[test]
    fn test_validate_url_rejects_rfc1918_192() {
        assert!(validate_url("http://192.168.1.1").is_err());
    }

    #[test]
    fn test_validate_url_rejects_ipv6_loopback() {
        assert!(validate_url("http://[::1]:8080").is_err());
    }

    #[test]
    fn test_validate_url_rejects_ipv6_link_local() {
        assert!(validate_url("http://[fe80::1]:8080").is_err());
    }

    #[test]
    fn test_validate_url_rejects_ipv6_unique_local() {
        assert!(validate_url("http://[fc00::1]:8080").is_err());
    }

    #[test]
    fn test_validate_url_rejects_localhost_hostname() {
        assert!(validate_url("http://localhost:9200").is_err());
        assert!(validate_url("http://LocalHost:9200").is_err());
    }

    #[test]
    fn test_validate_url_rejects_reserved_suffixes() {
        assert!(validate_url("http://vault.internal:8200").is_err());
        assert!(validate_url("http://service.local").is_err());
        assert!(validate_url("http://host.localhost").is_err());
    }

    #[test]
    fn test_validate_url_rejects_arpa_suffix() {
        assert!(validate_url("http://0.0.10.in-addr.arpa").is_err());
    }

    #[test]
    fn test_validate_url_rejects_malformed_input() {
        assert!(validate_url("not a url").is_err());
        assert!(validate_url("").is_err());
    }

    #[test]
    fn test_validate_url_escape_hatch_permits_private_networks() {
        std::env::set_var("VELESDB_MIGRATE_ALLOW_PRIVATE_NETWORKS", "1");
        assert!(validate_url("http://localhost:9200").is_ok());
        assert!(validate_url("http://127.0.0.1:6379").is_ok());
        assert!(validate_url("http://10.0.0.1").is_ok());
        // Scheme and userinfo checks remain active with the escape
        // hatch enabled — regression guard for defense-in-depth.
        assert!(validate_url("http://user:pass@localhost").is_err());
        assert!(validate_url("file:///etc/passwd").is_err());
        std::env::remove_var("VELESDB_MIGRATE_ALLOW_PRIVATE_NETWORKS");
    }

    #[test]
    fn test_validate_url_allows_public_redis_endpoint() {
        assert!(validate_url("rediss://redis.upstash.io:6379").is_ok());
    }

    #[test]
    fn test_create_http_client() {
        let client = create_http_client();
        // Client should be created successfully
        assert!(client.get("http://example.com").build().is_ok());
    }

    #[test]
    fn test_extract_id_from_number() {
        let val = Some(serde_json::json!(42));
        assert_eq!(extract_id_from_value(val), "42");
    }

    #[test]
    fn test_extract_id_from_string() {
        let val = Some(serde_json::json!("doc-123"));
        assert_eq!(extract_id_from_value(val), "doc-123");
    }

    #[test]
    fn test_extract_id_fallback_uuid() {
        let id = extract_id_from_value(None);
        // Should be a valid UUID v4 (36 chars with hyphens)
        assert_eq!(id.len(), 36);
    }

    #[test]
    fn test_format_count_some() {
        assert_eq!(format_count(Some(1000)), "1000");
    }

    #[test]
    fn test_format_count_none() {
        assert_eq!(format_count(None), "unknown");
    }
}
