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
