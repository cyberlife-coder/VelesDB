//! Shared VelesQL helpers for parse, validate, and error formatting.

use axum::{http::StatusCode, response::IntoResponse, Json};
use velesdb_core::velesql;

use crate::types::{
    QueryErrorDetail, QueryErrorResponse, VelesqlErrorDetail, VelesqlErrorResponse,
};

/// Parse a VelesQL query string and run validation, returning an error response on failure.
#[allow(clippy::result_large_err)]
pub(crate) fn parse_and_validate(
    query_str: &str,
) -> Result<velesdb_core::velesql::Query, axum::response::Response> {
    let parsed = velesql::Parser::parse(query_str).map_err(|e| velesql_parse_error(&e))?;

    velesql::QueryValidator::validate(&parsed).map_err(|e| {
        velesql_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "VELESQL_VALIDATION_ERROR",
            &e.to_string(),
            &e.suggestion,
            None,
        )
    })?;

    Ok(parsed)
}

/// Build a VelesQL parse error response from a `ParseError`.
pub(crate) fn velesql_parse_error(e: &velesql::ParseError) -> axum::response::Response {
    (
        StatusCode::BAD_REQUEST,
        Json(QueryErrorResponse {
            error: QueryErrorDetail {
                error_type: format!("{:?}", e.kind),
                message: e.message.clone(),
                position: e.position,
                query: e.fragment.clone(),
            },
        }),
    )
        .into_response()
}

/// Build a structured VelesQL error response.
pub(crate) fn velesql_error(
    status: StatusCode,
    code: &str,
    message: &str,
    hint: &str,
    details: Option<serde_json::Value>,
) -> axum::response::Response {
    (
        status,
        Json(VelesqlErrorResponse {
            error: VelesqlErrorDetail {
                code: code.to_string(),
                message: message.to_string(),
                hint: hint.to_string(),
                details,
            },
        }),
    )
        .into_response()
}

/// The status `/query` answers a statement that failed during execution
/// with: `422` for the client's errors, as the VelesQL contract has it, and
/// the canonical status of the engine's (`5xx`, from
/// [`http_status_for_error`](crate::handlers::helpers::http_status_for_error)),
/// so a storage fault or an internal error is no longer reported as the
/// client's. Which errors are the client's is core's
/// [`velesdb_core::Error::is_caller_error`].
pub(crate) fn velesql_execution_status(error: &velesdb_core::Error) -> StatusCode {
    if error.is_caller_error() {
        StatusCode::UNPROCESSABLE_ENTITY
    } else {
        crate::handlers::helpers::http_status_for_error(error)
    }
}

/// Build the VelesQL error response for a statement that failed during
/// execution, keeping the endpoint's `VELESQL_*` `code` and taking the status
/// from [`velesql_execution_status`].
pub(crate) fn velesql_execution_error(
    error: &velesdb_core::Error,
    code: &str,
    hint: &str,
) -> axum::response::Response {
    velesql_error(
        velesql_execution_status(error),
        code,
        &error.to_string(),
        hint,
        None,
    )
}

/// Build a 404 "collection not found" VelesQL error response.
pub(crate) fn velesql_collection_not_found(name: &str) -> axum::response::Response {
    velesql_error(
        StatusCode::NOT_FOUND,
        "VELESQL_COLLECTION_NOT_FOUND",
        &format!("Collection '{}' not found", name),
        "Create the collection first or correct the collection name",
        Some(serde_json::json!({ "collection": name })),
    )
}
