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
    // Exercises the policy core directly: mutating the process-global
    // environment from a test races sibling tests under the default
    // parallel runner.
    assert!(validate_url_with_policy("http://localhost:9200", true).is_ok());
    assert!(validate_url_with_policy("http://127.0.0.1:6379", true).is_ok());
    assert!(validate_url_with_policy("http://10.0.0.1", true).is_ok());
    // Scheme and userinfo checks remain active with the escape
    // hatch enabled — regression guard for defense-in-depth.
    assert!(validate_url_with_policy("http://user:pass@localhost", true).is_err());
    assert!(validate_url_with_policy("file:///etc/passwd", true).is_err());
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

#[test]
fn test_normalise_metric_maps_known_alias() {
    assert_eq!(normalise_metric("L2", &[("l2", "euclidean")]), "euclidean");
}

#[test]
fn test_normalise_metric_matches_case_insensitively() {
    assert_eq!(normalise_metric("Ip", &[("ip", "dot")]), "dot");
}

#[test]
fn test_normalise_metric_preserves_unknown_value() {
    assert_eq!(
        normalise_metric("manhattan", &[("l2", "euclidean")]),
        "manhattan"
    );
}

#[test]
fn test_normalise_metric_supports_multiple_aliases_to_same_canonical() {
    let aliases = [("l2", "euclidean"), ("vector_l2_ops", "euclidean")];
    assert_eq!(normalise_metric("vector_l2_ops", &aliases), "euclidean");
    assert_eq!(normalise_metric("l2", &aliases), "euclidean");
}
