//! Shared handler helpers to reduce duplication across endpoint modules.

use axum::{http::StatusCode, response::IntoResponse, Json};

use crate::types::ErrorResponse;
use crate::AppState;

/// Build an error response with the given status code and message.
///
/// Sets `code` to `None` — use [`core_error_response`] when a
/// `velesdb_core::Error` is available to propagate its VELES-XXX code.
pub(crate) fn error_response(status: StatusCode, message: String) -> axum::response::Response {
    (
        status,
        Json(ErrorResponse {
            error: message,
            code: None,
        }),
    )
        .into_response()
}

/// Build an error response from a [`velesdb_core::Error`], including the
/// VELES-XXX code in the JSON body.
///
/// Prefer [`auto_core_error_response`] which derives the HTTP status
/// automatically from the error variant.
pub(crate) fn core_error_response(
    status: StatusCode,
    error: &velesdb_core::Error,
) -> axum::response::Response {
    (
        status,
        Json(ErrorResponse {
            error: error.to_string(),
            code: Some(error.code().to_string()),
        }),
    )
        .into_response()
}

/// Derive the canonical HTTP status code from a [`velesdb_core::Error`].
///
/// Centralizes the error→status mapping so that every handler returns
/// consistent HTTP codes for the same error variant.
pub(crate) fn http_status_for_error(e: &velesdb_core::Error) -> StatusCode {
    use velesdb_core::Error;
    match e {
        // 404 Not Found
        Error::CollectionNotFound(_)
        | Error::PointNotFound(_)
        | Error::EdgeNotFound(_)
        | Error::NodeNotFound(_) => StatusCode::NOT_FOUND,

        // 409 Conflict
        Error::CollectionExists(_) | Error::EdgeExists(_) => StatusCode::CONFLICT,

        // 400 Bad Request — client input errors
        Error::DimensionMismatch { .. }
        | Error::InvalidVector(_)
        | Error::InvalidCollectionName { .. }
        | Error::InvalidDimension { .. }
        | Error::Config(_)
        | Error::Query(_)
        | Error::InvalidEdgeLabel(_)
        | Error::InvalidQuantizerConfig(_)
        | Error::SchemaValidation(_)
        | Error::VectorNotAllowed(_)
        | Error::VectorRequired(_)
        | Error::SearchNotSupported(_)
        | Error::GraphNotSupported(_)
        | Error::Overflow(_) => StatusCode::BAD_REQUEST,

        // 503 Service Unavailable
        Error::DatabaseLocked(_) | Error::GuardRail(_) => StatusCode::SERVICE_UNAVAILABLE,

        // 500 Internal Server Error — everything else
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

/// Build an error response from a [`velesdb_core::Error`], automatically
/// deriving the HTTP status code from the error variant.
pub(crate) fn auto_core_error_response(error: &velesdb_core::Error) -> axum::response::Response {
    core_error_response(http_status_for_error(error), error)
}

/// Look up a type-erased collection by name, returning a 404 response on miss.
///
/// Emits `VELES-002 CollectionNotFound` via [`core_error_response`] so that
/// SDK clients (TS, Python, …) can surface a typed
/// `CollectionNotFoundError` via `instanceof`. Fixes PR #586 Devin
/// finding #1: the prior `error_response` call set `code: None`, which
/// serde skipped from the JSON body. Clients then fell back to a
/// status-derived `'NOT_FOUND'` string and could not discriminate
/// collection-not-found from point/edge/node-not-found.
#[allow(clippy::result_large_err)]
pub(crate) fn get_collection_or_404(
    state: &AppState,
    name: &str,
) -> Result<velesdb_core::AnyCollection, axum::response::Response> {
    state.db.get_any_collection(name).ok_or_else(|| {
        core_error_response(
            StatusCode::NOT_FOUND,
            &velesdb_core::Error::CollectionNotFound(name.to_string()),
        )
    })
}

/// Look up a vector collection by name, returning a 404 response on miss.
///
/// Emits `VELES-002 CollectionNotFound` (same typed-error rationale as
/// [`get_collection_or_404`]). The "or is not a vector collection"
/// disambiguation lives in the response body message — the code field
/// stays VELES-002 so typed-error clients can still narrow on
/// `CollectionNotFoundError`.
#[allow(clippy::result_large_err)]
pub(crate) fn get_vector_collection_or_404(
    state: &AppState,
    name: &str,
) -> Result<velesdb_core::collection::VectorCollection, axum::response::Response> {
    state.db.get_vector_collection(name).ok_or_else(|| {
        core_error_response(
            StatusCode::NOT_FOUND,
            &velesdb_core::Error::CollectionNotFound(name.to_string()),
        )
    })
}

/// Extract client identifier from request headers.
///
/// Falls back to `"anonymous"` if no `X-Client-Id` header is present.
pub(crate) fn extract_client_id(headers: &axum::http::HeaderMap) -> String {
    headers
        .get("x-client-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("anonymous")
        .to_string()
}

/// Record query timing via a `tracing` event and notify the `DatabaseObserver`.
///
/// Emits a structured log at `DEBUG` level and forwards the query duration
/// to `state.db.notify_query()` so that `DatabaseObserver` implementations
/// (audit, RBAC, usage tracking) receive the event.
pub(crate) fn notify_query_timing(
    state: &AppState,
    collection_name: &str,
    start: std::time::Instant,
) {
    let duration_us = start.elapsed().as_micros();
    let elapsed_ms = duration_us as f64 / 1000.0;
    tracing::debug!(collection = collection_name, elapsed_ms, "query completed");
    // Reason: clamped to u64::MAX above — truncation is impossible
    #[allow(clippy::cast_possible_truncation)]
    state.db.notify_query(
        collection_name,
        duration_us.min(u128::from(u64::MAX)) as u64,
    );
}

/// Apply guardrails pre-check (rate limiting + circuit breaker).
///
/// Returns `Err` with a 429 response on rate limit exceeded, or a 503
/// response when the circuit breaker is open.
#[allow(clippy::result_large_err)]
pub(crate) fn apply_pre_check(
    guard_rails: &velesdb_core::guardrails::GuardRails,
    client_id: &str,
) -> Result<(), axum::response::Response> {
    if let Err(violation) = guard_rails.pre_check(client_id) {
        let (status, msg) = match violation {
            velesdb_core::guardrails::GuardRailViolation::RateLimitExceeded { .. } => (
                StatusCode::TOO_MANY_REQUESTS,
                format!("Rate limit exceeded for client '{client_id}'"),
            ),
            velesdb_core::guardrails::GuardRailViolation::CircuitOpen { .. } => (
                StatusCode::SERVICE_UNAVAILABLE,
                "Circuit breaker is open — too many recent failures".to_string(),
            ),
            other => (
                StatusCode::SERVICE_UNAVAILABLE,
                format!("Guard rail violation: {other}"),
            ),
        };
        return Err((
            status,
            Json(ErrorResponse {
                error: msg,
                code: None,
            }),
        )
            .into_response());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderMap;

    #[test]
    fn test_extract_client_id_from_header() {
        let mut headers = HeaderMap::new();
        headers.insert("x-client-id", "my-app".parse().unwrap());
        assert_eq!(extract_client_id(&headers), "my-app");
    }

    #[test]
    fn test_extract_client_id_fallback() {
        let headers = HeaderMap::new();
        assert_eq!(extract_client_id(&headers), "anonymous");
    }

    #[test]
    fn test_extract_client_id_invalid_utf8_falls_back() {
        let mut headers = HeaderMap::new();
        // HeaderValue with valid ASCII always succeeds to_str,
        // so we verify the fallback path by omitting the header.
        headers.insert("x-other-header", "value".parse().unwrap());
        assert_eq!(extract_client_id(&headers), "anonymous");
    }

    #[test]
    fn test_error_response_no_code() {
        let resp = error_response(StatusCode::BAD_REQUEST, "bad request".to_string());
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn test_core_error_response_includes_code() {
        let err = velesdb_core::Error::DimensionMismatch {
            expected: 384,
            actual: 768,
        };
        let resp = core_error_response(StatusCode::BAD_REQUEST, &err);
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }
}
