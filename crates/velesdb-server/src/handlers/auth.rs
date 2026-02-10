//! API key authentication middleware for VelesDB server.
//!
//! When `VELESDB_API_KEY` is set, all endpoints except `/health` and `/swagger-ui`
//! require `Authorization: Bearer <key>`. When unset, the server runs in dev mode.

use axum::{
    body::Body,
    extract::{Request, State},
    http::{header, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use std::sync::Arc;

use crate::types::ErrorResponse;
use crate::AppState;

/// Constant-time byte comparison to prevent timing attacks.
///
/// Returns `true` if both slices are equal, using a fixed-time algorithm
/// that does NOT short-circuit on the first mismatch.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    // Reason: XOR accumulator ensures we always iterate all bytes.
    // A non-zero result means at least one byte differed.
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Axum middleware: validate API key from `Authorization: Bearer <key>` header.
///
/// - If `AppState.api_key` is `None` → dev mode, pass through.
/// - If the request path is exempt (health, swagger) → pass through.
/// - Otherwise, validate the Bearer token with constant-time comparison.
pub async fn auth_middleware(
    State(state): State<Arc<AppState>>,
    request: Request<Body>,
    next: Next,
) -> Response {
    // Dev mode: no API key configured → pass through
    let expected_key = match &state.api_key {
        Some(key) => key,
        None => return next.run(request).await,
    };

    // Exempt paths: health check, swagger UI, OpenAPI docs
    let path = request.uri().path();
    if path == "/health" || path.starts_with("/swagger-ui") || path.starts_with("/api-docs") {
        return next.run(request).await;
    }

    // Extract Authorization header
    let auth_header = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());

    let token = match auth_header {
        Some(h) if h.starts_with("Bearer ") => &h[7..],
        _ => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(ErrorResponse {
                    error: "Missing or malformed Authorization header. Expected: Bearer <api_key>"
                        .to_string(),
                }),
            )
                .into_response();
        }
    };

    // Constant-time comparison
    if !constant_time_eq(token.as_bytes(), expected_key.as_bytes()) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(ErrorResponse {
                error: "Invalid API key".to_string(),
            }),
        )
            .into_response();
    }

    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constant_time_eq_equal() {
        assert!(constant_time_eq(b"secret123", b"secret123"));
    }

    #[test]
    fn test_constant_time_eq_different() {
        assert!(!constant_time_eq(b"secret123", b"secret456"));
    }

    #[test]
    fn test_constant_time_eq_different_lengths() {
        assert!(!constant_time_eq(b"short", b"longer_string"));
    }

    #[test]
    fn test_constant_time_eq_empty() {
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn test_constant_time_eq_single_bit_diff() {
        // 'a' = 0x61, 'b' = 0x62 — differ by 1 bit
        assert!(!constant_time_eq(b"a", b"b"));
    }
}
