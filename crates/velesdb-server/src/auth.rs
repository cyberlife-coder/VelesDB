//! API key authentication middleware.
//!
//! When `api_keys` is non-empty, all requests except those to public paths
//! (e.g. `GET /health`) must include a valid `Authorization: Bearer <key>` header.
//! When `api_keys` is empty, authentication is disabled (local dev mode).

use axum::{
    body::Body,
    extract::Request,
    http::{header, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use std::sync::Arc;

/// Constant-time byte comparison to prevent timing side-channel attacks.
///
/// Compares two byte slices in constant time relative to the length of `a`.
/// Returns `true` only when both slices have equal length and identical contents.
/// Uses XOR-and-fold so that the comparison does not short-circuit on the first
/// differing byte.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        // Length mismatch leaks the length difference, but not the key contents.
        // This is acceptable: an attacker already controls `b` (the submitted
        // token) and can trivially discover the expected length via other means
        // (e.g. documentation). The critical property is that *content* is never
        // leaked through timing.
        return false;
    }

    let mut acc: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        acc |= x ^ y;
    }
    acc == 0
}

/// Checks whether `token` matches any configured API key in constant time.
///
/// Iterates over **all** keys regardless of early matches to avoid leaking
/// which key (if any) was correct through timing differences.
fn any_key_matches(keys: &[String], token: &str) -> bool {
    let token_bytes = token.as_bytes();
    let mut matched = false;
    for key in keys {
        if constant_time_eq(key.as_bytes(), token_bytes) {
            matched = true;
        }
        // Do NOT early-return — iterate all keys unconditionally.
    }
    matched
}

/// Shared authentication state injected into the middleware.
#[derive(Debug, Clone)]
pub struct AuthState {
    /// Allowed API keys. Empty means auth is disabled.
    pub api_keys: Arc<Vec<String>>,
}

impl AuthState {
    /// Create a new `AuthState` from a list of API keys.
    pub fn new(api_keys: Vec<String>) -> Self {
        Self {
            api_keys: Arc::new(api_keys),
        }
    }

    /// Returns `true` when authentication is enabled.
    pub fn auth_enabled(&self) -> bool {
        !self.api_keys.is_empty()
    }
}

/// Paths that bypass authentication (both legacy and `/v1/` versioned).
///
/// `/health` and `/ready` are the only genuinely public endpoints — they
/// expose no internal state beyond liveness booleans and are needed by
/// container orchestrators that cannot carry authentication headers.
///
/// `/metrics` is **not** in this list. The Prometheus metrics endpoint
/// returns detailed operational data (collection counts, cache hit
/// rates, query latencies, per-collection sizes, WAL depths) that
/// constitutes an information leak when the surrounding REST API is
/// protected by API keys. The fix for F-02 moves `/metrics` behind the
/// same API key gate as the rest of the REST surface: scrapers must
/// present `Authorization: Bearer <key>` like any other client.
///
/// Operators who need a dedicated scraping credential can provision a
/// distinct API key for the Prometheus scraper and rotate it
/// independently.
fn is_public_path(path: &str) -> bool {
    matches!(path, "/health" | "/ready" | "/v1/health" | "/v1/ready")
}

/// Extract the Bearer token from the Authorization header value.
fn extract_bearer_token(header_value: &str) -> Option<&str> {
    let trimmed = header_value.trim();
    if trimmed.len() > 7 && trimmed[..7].eq_ignore_ascii_case("bearer ") {
        let token = trimmed[7..].trim();
        if token.is_empty() {
            None
        } else {
            Some(token)
        }
    } else {
        None
    }
}

/// Axum middleware function for API key authentication.
///
/// Use with `axum::middleware::from_fn_with_state`.
pub async fn auth_middleware(
    axum::extract::State(state): axum::extract::State<AuthState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    // Skip auth if disabled (no keys configured)
    if !state.auth_enabled() {
        return next.run(request).await;
    }

    // Skip auth for public paths
    if is_public_path(request.uri().path()) {
        return next.run(request).await;
    }

    // Extract and validate Bearer token
    let auth_header = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());

    match auth_header {
        Some(value) => match extract_bearer_token(value) {
            Some(token) if any_key_matches(&state.api_keys, token) => next.run(request).await,
            Some(_) => unauthorized_response("invalid API key"),
            None => {
                unauthorized_response("invalid Authorization header format, expected: Bearer <key>")
            }
        },
        None => unauthorized_response("missing Authorization header"),
    }
}

/// Build a 401 Unauthorized JSON response.
fn unauthorized_response(message: &str) -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(serde_json::json!({
            "error": "Unauthorized",
            "message": message
        })),
    )
        .into_response()
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auth_state_disabled_when_empty() {
        let state = AuthState::new(vec![]);
        assert!(!state.auth_enabled());
    }

    #[test]
    fn test_auth_state_enabled_with_keys() {
        let state = AuthState::new(vec!["key1".to_string()]);
        assert!(state.auth_enabled());
    }

    #[test]
    fn test_is_public_path_health() {
        assert!(is_public_path("/health"));
    }

    #[test]
    fn test_is_public_path_ready() {
        assert!(is_public_path("/ready"));
    }

    #[test]
    fn test_is_public_path_metrics_is_protected() {
        // F-02: /metrics must not bypass authentication — it leaks
        // operational details about the running database.
        assert!(!is_public_path("/metrics"));
        assert!(!is_public_path("/v1/metrics"));
    }

    #[test]
    fn test_is_public_path_versioned_health() {
        assert!(is_public_path("/v1/health"));
    }

    #[test]
    fn test_is_public_path_versioned_ready() {
        assert!(is_public_path("/v1/ready"));
    }

    #[test]
    fn test_is_public_path_other() {
        assert!(!is_public_path("/collections"));
        assert!(!is_public_path("/query"));
        assert!(!is_public_path("/health/extra"));
        assert!(!is_public_path("/v1/collections"));
    }

    #[test]
    fn test_extract_bearer_token_valid() {
        assert_eq!(extract_bearer_token("Bearer my-key"), Some("my-key"));
        assert_eq!(extract_bearer_token("bearer my-key"), Some("my-key"));
        assert_eq!(extract_bearer_token("BEARER my-key"), Some("my-key"));
        assert_eq!(extract_bearer_token("  Bearer  my-key  "), Some("my-key"));
    }

    #[test]
    fn test_extract_bearer_token_invalid() {
        assert_eq!(extract_bearer_token("Basic abc123"), None);
        assert_eq!(extract_bearer_token("my-key"), None);
        assert_eq!(extract_bearer_token("Bearer"), None);
        assert_eq!(extract_bearer_token(""), None);
    }

    #[test]
    fn test_extract_bearer_token_whitespace_only() {
        assert_eq!(extract_bearer_token("Bearer   "), None);
    }

    // ========================================================================
    // Constant-time comparison tests
    // ========================================================================

    #[test]
    fn test_constant_time_eq_identical() {
        assert!(constant_time_eq(b"secret-key-42", b"secret-key-42"));
    }

    #[test]
    fn test_constant_time_eq_different_content() {
        assert!(!constant_time_eq(b"secret-key-42", b"secret-key-43"));
    }

    #[test]
    fn test_constant_time_eq_different_length() {
        assert!(!constant_time_eq(b"short", b"longer-key"));
    }

    #[test]
    fn test_constant_time_eq_empty() {
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn test_any_key_matches_found() {
        let keys = vec!["key-a".to_string(), "key-b".to_string()];
        assert!(any_key_matches(&keys, "key-b"));
    }

    #[test]
    fn test_any_key_matches_not_found() {
        let keys = vec!["key-a".to_string(), "key-b".to_string()];
        assert!(!any_key_matches(&keys, "key-c"));
    }

    #[test]
    fn test_any_key_matches_empty_keys() {
        let keys: Vec<String> = vec![];
        assert!(!any_key_matches(&keys, "anything"));
    }
}
