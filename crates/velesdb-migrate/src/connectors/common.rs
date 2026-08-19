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
    let allow_private = std::env::var("VELESDB_MIGRATE_ALLOW_PRIVATE_NETWORKS")
        .ok()
        .filter(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .is_some();
    validate_url_with_policy(input, allow_private)
}

/// Policy-parameterised core of [`validate_url`]: `allow_private` bypasses
/// checks (4) and (5) only, exactly like the environment escape hatch.
///
/// Taking the policy as a parameter keeps the function testable without
/// mutating process-global environment state from tests.
fn validate_url_with_policy(input: &str, allow_private: bool) -> Result<()> {
    // Delegate RFC 3986 parsing to the `url` crate.
    let parsed =
        url::Url::parse(input).map_err(|e| Error::Config(format!("Invalid URL '{input}': {e}")))?;

    reject_scheme_and_userinfo(&parsed, input)?;
    let host = require_host(&parsed, input)?;

    // Local development escape hatch: bypass checks (4) and (5) only.
    if allow_private {
        return Ok(());
    }

    // (4) and (5) Private-range and reserved-hostname rejection.
    reject_unsafe_host(&host, input)
}

/// Checks (1) and (2) documented on [`validate_url`]: scheme allowlist and
/// userinfo rejection (credential smuggling / parser-confusion attacks where
/// a crafted `user@host` component overrides the caller's intended target).
fn reject_scheme_and_userinfo(parsed: &url::Url, input: &str) -> Result<()> {
    let scheme = parsed.scheme();
    if !ALLOWED_SCHEMES.contains(&scheme) {
        return Err(Error::Config(format!(
            "Disallowed URL scheme '{scheme}' in '{input}'. \
             Allowed: {}",
            ALLOWED_SCHEMES.join(", ")
        )));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(Error::Config(format!(
            "URL '{input}' must not contain userinfo (user:pass@host). \
             Pass credentials via the connector's explicit auth config."
        )));
    }
    Ok(())
}

/// Check (3) documented on [`validate_url`]: host presence and non-emptiness.
fn require_host<'a>(parsed: &'a url::Url, input: &str) -> Result<url::Host<&'a str>> {
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
    Ok(host)
}

/// Rejects hosts that resolve to private, loopback, link-local, or
/// cloud-metadata endpoints. Implements checks (4) and (5) documented on
/// [`validate_url`].
///
/// Returns `Ok(())` if the host is publicly routable, `Err(Error::Config)`
/// otherwise.
fn reject_unsafe_host(host: &url::Host<&str>, input: &str) -> Result<()> {
    match host {
        url::Host::Ipv4(ip) => reject_unsafe_ipv4(*ip, input),
        url::Host::Ipv6(ip) => reject_unsafe_ipv6(*ip, input),
        url::Host::Domain(name) => reject_reserved_domain(name, input),
    }
}

/// Rejects IPv4 hosts in loopback, RFC 1918 private, link-local ranges, or the
/// cloud metadata endpoint. Helper for [`reject_unsafe_host`].
fn reject_unsafe_ipv4(ip: std::net::Ipv4Addr, input: &str) -> Result<()> {
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
    Ok(())
}

/// Rejects IPv6 loopback/unspecified, link-local (fe80::/10), and
/// unique-local (fc00::/7) hosts. Helper for [`reject_unsafe_host`].
fn reject_unsafe_ipv6(ip: std::net::Ipv6Addr, input: &str) -> Result<()> {
    if ip.is_loopback() || ip.is_unspecified() {
        return Err(Error::Config(format!(
            "URL '{input}' targets an IPv6 loopback or unspecified \
             address ({ip})"
        )));
    }
    // Detect fe80::/10 (link-local) and fc00::/7 (unique local) via
    // their IPv6 prefix masks: `Ipv6Addr::is_unique_local` and
    // `is_unicast_link_local` are still nightly-only behind
    // `feature(ip)` at our MSRV (1.89), so we match the prefixes by
    // hand to keep the connector buildable on stable.
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
    Ok(())
}

/// Rejects reserved hostnames that name on-host or internal-only services.
/// Helper for [`reject_unsafe_host`].
fn reject_reserved_domain(name: &str, input: &str) -> Result<()> {
    let lower = name.to_ascii_lowercase();
    // Reject reserved suffixes that name on-host or internal-only
    // services. These cover both standards-reserved labels
    // (RFC 6761 `.localhost`, RFC 8375 `.home.arpa`) and common
    // private-DNS conventions (`.internal`, `.local`).
    const RESERVED_SUFFIXES: &[&str] = &["localhost", ".localhost", ".local", ".internal", ".arpa"];
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
    let Value::Object(map) = source else {
        return HashMap::new();
    };
    extract_payload_from_hashmap(map, excluded_fields, included_fields)
}

/// Extracts payload fields from a pre-parsed `HashMap<String, Value>`.
///
/// Identical filtering semantics to [`extract_payload_from_object`] but
/// operates directly on a `HashMap`, avoiding a round-trip through
/// `serde_json::Value::Object` when the caller already has the map form.
pub fn extract_payload_from_hashmap<'a>(
    source: impl IntoIterator<Item = (&'a String, &'a Value)>,
    excluded_fields: &[&str],
    included_fields: &[String],
) -> HashMap<String, Value> {
    source
        .into_iter()
        .filter(|(key, _)| !excluded_fields.contains(&key.as_str()))
        .filter(|(key, _)| included_fields.is_empty() || included_fields.contains(key))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
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

/// Lowercases `raw` and rewrites it to a canonical distance-metric name
/// using `aliases`, falling back to the lowercased value verbatim.
///
/// Every source connector exposes its own vendor-specific metric labels
/// (Milvus's `L2`/`IP`, Qdrant's `Euclid`, pgvector's `vector_l2_ops`, ...)
/// and normalises them to the VelesDB core vocabulary (`cosine`, `dot`,
/// `euclidean`, `hamming`, `jaccard`) so `Pipeline::check_metric_fidelity`
/// can compare a source metric against a destination collection's metric.
/// `aliases` lists `(vendor_label, canonical_name)` pairs matched against
/// the lowercased input; unknown labels are lowercased and returned
/// verbatim so mismatch errors stay actionable rather than being masked.
pub fn normalise_metric(raw: &str, aliases: &[(&str, &str)]) -> String {
    let lower = raw.to_ascii_lowercase();
    aliases
        .iter()
        .find(|(alias, _)| *alias == lower)
        .map_or(lower, |(_, canonical)| (*canonical).to_string())
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
#[path = "common_tests.rs"]
mod tests;
