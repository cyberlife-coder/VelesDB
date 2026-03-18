//! Batch search handler: multiple vector queries in a single request.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use std::sync::Arc;

use crate::types::{
    BatchSearchRequest, BatchSearchResponse, ErrorResponse, SearchResponse, SearchResultResponse,
};
use crate::AppState;

use super::pipeline::{actionable_search_error, record_circuit_breaker, validate_query_dimension};
use crate::handlers::helpers::{
    apply_pre_check, extract_client_id, get_vector_collection_or_404, notify_query_timing,
};

/// Batch search for multiple vectors.
#[utoipa::path(
    post,
    path = "/collections/{name}/search/batch",
    tag = "search",
    params(
        ("name" = String, Path, description = "Collection name")
    ),
    request_body = BatchSearchRequest,
    responses(
        (status = 200, description = "Batch search results", body = BatchSearchResponse),
        (status = 404, description = "Collection not found", body = ErrorResponse),
        (status = 400, description = "Invalid request", body = ErrorResponse)
    )
)]
#[allow(clippy::unused_async)]
pub async fn batch_search(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Path(name): Path<String>,
    Json(req): Json<BatchSearchRequest>,
) -> impl IntoResponse {
    let start = std::time::Instant::now();
    state.onboarding_metrics.record_search_request();

    let collection = match get_vector_collection_or_404(&state, &name) {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    let client_id = extract_client_id(&headers);
    if let Err(resp) = apply_pre_check(collection.guard_rails(), &client_id) {
        return resp;
    }

    if let Err(resp) = validate_batch_dimensions(&state, &name, &collection, &req) {
        return resp;
    }

    let filters = match parse_batch_filters(&state, &req) {
        Ok(f) => f,
        Err(resp) => return resp,
    };

    let queries: Vec<&[f32]> = req.searches.iter().map(|s| s.vector.as_slice()).collect();
    let max_top_k = req.searches.iter().map(|s| s.top_k).max().unwrap_or(10);

    let batch_result = collection.search_batch_with_filters(&queries, max_top_k, &filters);
    record_circuit_breaker(&collection, &batch_result);

    let all_results = match batch_result {
        Ok(batch_results) => build_batch_responses(&state, batch_results, &req),
        Err(e) => {
            return (StatusCode::BAD_REQUEST, Json(actionable_search_error(&e))).into_response();
        }
    };

    let timing_ms = start.elapsed().as_secs_f64() * 1000.0;
    notify_query_timing(&state, &name, start);

    Json(BatchSearchResponse {
        results: all_results,
        timing_ms,
    })
    .into_response()
}

/// Validate that every query vector in a batch request matches the collection dimension.
#[allow(clippy::result_large_err)]
fn validate_batch_dimensions(
    state: &AppState,
    name: &str,
    collection: &velesdb_core::collection::VectorCollection,
    req: &BatchSearchRequest,
) -> Result<(), axum::response::Response> {
    let expected_dimension = collection.config().dimension;
    for (idx, search) in req.searches.iter().enumerate() {
        if let Err(error) =
            validate_query_dimension(state, name, expected_dimension, &search.vector)
        {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!("Invalid query at index {idx}: {}", error.error),
                }),
            )
                .into_response());
        }
    }
    Ok(())
}

/// Parse filters from each search in a batch request.
#[allow(clippy::result_large_err)]
fn parse_batch_filters(
    state: &AppState,
    req: &BatchSearchRequest,
) -> Result<Vec<Option<velesdb_core::Filter>>, axum::response::Response> {
    let mut filters: Vec<Option<velesdb_core::Filter>> = Vec::with_capacity(req.searches.len());
    for (idx, search) in req.searches.iter().enumerate() {
        if let Some(filter_json) = &search.filter {
            match serde_json::from_value(filter_json.clone()) {
                Ok(filter) => filters.push(Some(filter)),
                Err(e) => {
                    state.onboarding_metrics.record_filter_parse_error();
                    return Err((
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: format!(
                                "Invalid filter at index {idx}: {e}. Hint: validate filter syntax and start with a broader query before reintroducing strict filters."
                            ),
                        }),
                    )
                        .into_response());
                }
            }
        } else {
            filters.push(None);
        }
    }
    Ok(filters)
}

/// Convert batch search results into response objects, recording metrics for empty results.
fn build_batch_responses(
    state: &AppState,
    batch_results: Vec<Vec<velesdb_core::SearchResult>>,
    req: &BatchSearchRequest,
) -> Vec<SearchResponse> {
    let empty_count = batch_results
        .iter()
        .filter(|results| results.is_empty())
        .count();
    for _ in 0..empty_count {
        state.onboarding_metrics.record_empty_search_results();
    }
    debug_assert_eq!(
        batch_results.len(),
        req.searches.len(),
        "search_batch_with_filters must return one result-vec per query"
    );
    batch_results
        .into_iter()
        .zip(req.searches.iter())
        .map(|(results, search)| SearchResponse {
            results: results
                .into_iter()
                .take(search.top_k)
                .map(|r| SearchResultResponse {
                    id: r.point.id,
                    score: r.score,
                    payload: r.point.payload,
                })
                .collect(),
        })
        .collect()
}
