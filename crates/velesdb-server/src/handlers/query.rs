//! VelesQL query execution handler.

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use std::sync::Arc;

use crate::types::{
    ErrorResponse, QueryErrorDetail, QueryErrorResponse, QueryRequest, QueryResponse,
    SearchResultResponse,
};
use crate::AppState;
use velesdb_core::velesql;

/// Execute a VelesQL query.
#[utoipa::path(
    post,
    path = "/query",
    tag = "query",
    request_body = QueryRequest,
    responses(
        (status = 200, description = "Query results", body = QueryResponse),
        (status = 400, description = "Query syntax error", body = QueryErrorResponse),
        (status = 404, description = "Collection not found", body = ErrorResponse)
    )
)]
#[allow(clippy::unused_async)]
pub async fn query(
    State(state): State<Arc<AppState>>,
    Json(req): Json<QueryRequest>,
) -> impl IntoResponse {
    let start = std::time::Instant::now();

    let parsed = match velesql::Parser::parse(&req.query) {
        Ok(q) => q,
        Err(e) => {
            return (
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
    };

    let select = &parsed.select;

    let collection = match state.db.get_collection(&select.from) {
        Some(c) => c,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: format!("Collection '{}' not found", select.from),
                }),
            )
                .into_response()
        }
    };

    let results = match collection.execute_query(&parsed, &req.params) {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            )
                .into_response()
        }
    };

    let timing_ms = start.elapsed().as_secs_f64() * 1000.0;
    let rows_returned = results.len();

    Json(QueryResponse {
        results: results
            .into_iter()
            .map(|r| SearchResultResponse {
                id: r.point.id,
                score: r.score,
                payload: r.point.payload,
            })
            .collect(),
        timing_ms,
        rows_returned,
    })
    .into_response()
}
