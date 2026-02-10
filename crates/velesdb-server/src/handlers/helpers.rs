//! Shared handler helpers for VelesDB REST API.
//!
//! Provides common patterns used across all handlers to reduce duplication
//! and ensure consistent error responses.

use axum::{http::StatusCode, Json};
use velesdb_core::Collection;

use crate::types::ErrorResponse;
use crate::AppState;

/// Look up a collection by name or return HTTP 404.
///
/// Replaces the duplicated `match state.db.get_collection(&name)` pattern
/// found in every handler.
///
/// # Errors
///
/// Returns `(404, ErrorResponse)` if the collection does not exist.
pub fn get_collection_or_404(
    state: &AppState,
    name: &str,
) -> Result<Collection, (StatusCode, Json<ErrorResponse>)> {
    state.db.get_collection(name).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("Collection '{name}' not found"),
            }),
        )
    })
}

/// Build an internal server error response without leaking implementation details.
///
/// Logs the full error server-side via `tracing::error!` and returns a generic
/// message to the client. This prevents exposing panic backtraces, task join
/// errors, or internal state to API consumers.
pub fn internal_error(
    context: &str,
    err: &dyn std::fmt::Display,
) -> (StatusCode, Json<ErrorResponse>) {
    tracing::error!(%context, error = %err, "Internal server error");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorResponse {
            error: format!("{context}: internal error"),
        }),
    )
}

/// Maximum allowed value for `top_k` to prevent excessive memory allocation.
const MAX_TOP_K: usize = 10_000;

/// Build a 400 Bad Request response with the given message.
pub fn bad_request(msg: impl Into<String>) -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::BAD_REQUEST,
        Json(ErrorResponse { error: msg.into() }),
    )
}

/// Validate `top_k` is within bounds (1..=10000).
///
/// # Errors
///
/// Returns `(400, ErrorResponse)` if `top_k` is 0 or exceeds `MAX_TOP_K`.
pub fn validate_top_k(top_k: usize) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    if top_k == 0 {
        return Err(bad_request("top_k must be at least 1"));
    }
    if top_k > MAX_TOP_K {
        return Err(bad_request(format!(
            "top_k must be at most {MAX_TOP_K}, got {top_k}"
        )));
    }
    Ok(())
}

/// Validate that a query string is not empty or whitespace-only.
///
/// # Errors
///
/// Returns `(400, ErrorResponse)` if the query is blank.
pub fn validate_query_non_empty(query: &str) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    if query.trim().is_empty() {
        return Err(bad_request("query must not be empty"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use velesdb_core::Database;

    #[test]
    fn test_get_collection_or_404_not_found() {
        let dir = "test_helpers_404";
        let db = Database::open(dir).expect("db");
        let state = AppState { db, api_key: None };
        let result = get_collection_or_404(&state, "nonexistent");
        match result {
            Err((status, Json(body))) => {
                assert_eq!(status, StatusCode::NOT_FOUND);
                assert!(body.error.contains("nonexistent"));
            }
            Ok(_) => panic!("Expected 404 error for nonexistent collection"),
        }
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn test_validate_top_k_zero_rejected() {
        let result = validate_top_k(0);
        match result {
            Err((status, Json(body))) => {
                assert_eq!(status, StatusCode::BAD_REQUEST);
                assert!(body.error.contains("at least 1"));
            }
            Ok(()) => panic!("Expected error for top_k=0"),
        }
    }

    #[test]
    fn test_validate_top_k_exceeds_max() {
        let result = validate_top_k(10_001);
        match result {
            Err((status, Json(body))) => {
                assert_eq!(status, StatusCode::BAD_REQUEST);
                assert!(body.error.contains("10000"));
            }
            Ok(()) => panic!("Expected error for top_k > 10000"),
        }
    }

    #[test]
    fn test_validate_top_k_valid() {
        assert!(validate_top_k(1).is_ok());
        assert!(validate_top_k(100).is_ok());
        assert!(validate_top_k(10_000).is_ok());
    }

    #[test]
    fn test_validate_query_empty_rejected() {
        assert!(validate_query_non_empty("").is_err());
        assert!(validate_query_non_empty("   ").is_err());
        assert!(validate_query_non_empty("\t\n").is_err());
    }

    #[test]
    fn test_validate_query_non_empty_valid() {
        assert!(validate_query_non_empty("hello").is_ok());
        assert!(validate_query_non_empty(" a ").is_ok());
    }

    #[test]
    fn test_internal_error_does_not_leak_details() {
        let detail = "JoinError: task panicked with sensitive data";
        let (status, Json(body)) = internal_error("Search", &detail);
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(body.error.contains("internal error"));
        // Reason: must NOT contain the raw panic message
        assert!(!body.error.contains("panicked"));
        assert!(!body.error.contains("sensitive"));
    }
}
