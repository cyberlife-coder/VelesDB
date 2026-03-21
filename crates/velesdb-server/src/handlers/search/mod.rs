//! Search handlers for vector similarity, text, and hybrid search.

pub(crate) mod batch;
pub(crate) mod multi;
mod pipeline;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use std::sync::Arc;
use velesdb_core::collection::VectorCollection;

use crate::types::{
    HybridSearchRequest, SearchIdsResponse, SearchRequest, SearchResponse, TextSearchRequest,
};
use crate::AppState;

use super::helpers::{apply_pre_check, extract_client_id, get_vector_collection_or_404};
use pipeline::{
    execute_search_request, finish_search_ids_with_cb, finish_search_with_cb,
    finish_search_with_status, parse_filter_or_400, validate_query_dimension,
};

#[allow(unused_imports)]
pub use batch::__path_batch_search;
pub use batch::batch_search;
#[allow(unused_imports)]
pub use multi::__path_multi_query_search;
pub use multi::multi_query_search;

/// Shared search preamble: record metric, resolve collection, check guard rails.
///
/// Returns `Ok(collection)` or `Err(response)` on failure.
#[allow(clippy::result_large_err)]
fn search_preamble(
    state: &AppState,
    name: &str,
    headers: &axum::http::HeaderMap,
) -> Result<VectorCollection, axum::response::Response> {
    state.onboarding_metrics.record_search_request();
    let collection = get_vector_collection_or_404(state, name)?;
    let client_id = extract_client_id(headers);
    apply_pre_check(collection.guard_rails(), &client_id)?;
    Ok(collection)
}

/// Executes the full search pipeline and records circuit-breaker on failure.
///
/// Shared by `/search` and `/search/ids` (both accept `SearchRequest`).
#[allow(clippy::result_large_err)]
fn execute_with_cb(
    state: &AppState,
    name: &str,
    collection: &VectorCollection,
    req: &mut SearchRequest,
) -> Result<velesdb_core::Result<Vec<velesdb_core::SearchResult>>, axum::response::Response> {
    execute_search_request(state, name, collection, req).inspect_err(|_| {
        collection.guard_rails().circuit_breaker.record_failure();
    })
}

/// Search for similar vectors.
///
/// Auto-detects search mode:
/// - **Dense**: `vector` only (existing behavior)
/// - **Sparse**: `sparse_vector` only
/// - **Hybrid**: both `vector` and `sparse_vector` (fused via RRF/RSF)
#[utoipa::path(
    post,
    path = "/collections/{name}/search",
    tag = "search",
    params(
        ("name" = String, Path, description = "Collection name")
    ),
    request_body = SearchRequest,
    responses(
        (status = 200, description = "Search results", body = SearchResponse),
        (status = 404, description = "Collection not found", body = crate::types::ErrorResponse),
        (status = 400, description = "Invalid request", body = crate::types::ErrorResponse)
    )
)]
#[allow(clippy::unused_async)]
pub async fn search(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Path(name): Path<String>,
    Json(mut req): Json<SearchRequest>,
) -> impl IntoResponse {
    let start = std::time::Instant::now();

    let collection = match search_preamble(&state, &name, &headers) {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    let search_result = match execute_with_cb(&state, &name, &collection, &mut req) {
        Ok(r) => r,
        Err(resp) => return resp,
    };

    finish_search_with_cb(&state, &name, start, &collection, search_result)
}

/// Search using BM25 full-text search.
#[utoipa::path(
    post,
    path = "/collections/{name}/search/text",
    tag = "search",
    params(
        ("name" = String, Path, description = "Collection name")
    ),
    request_body = TextSearchRequest,
    responses(
        (status = 200, description = "Text search results", body = SearchResponse),
        (status = 404, description = "Collection not found", body = crate::types::ErrorResponse)
    )
)]
#[allow(clippy::unused_async)]
pub async fn text_search(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Path(name): Path<String>,
    Json(req): Json<TextSearchRequest>,
) -> impl IntoResponse {
    let collection = match search_preamble(&state, &name, &headers) {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    let start = std::time::Instant::now();

    let search_result = if let Some(ref filter_json) = req.filter {
        let filter = match parse_filter_or_400(filter_json, &state.onboarding_metrics) {
            Ok(f) => f,
            Err(resp) => return resp,
        };
        collection.text_search_with_filter(&req.query, req.top_k, &filter)
    } else {
        collection.text_search(&req.query, req.top_k)
    };

    finish_search_with_status(
        &state,
        &name,
        start,
        &collection,
        StatusCode::INTERNAL_SERVER_ERROR,
        search_result,
    )
}

/// Hybrid search combining vector similarity and BM25 text search.
#[utoipa::path(
    post,
    path = "/collections/{name}/search/hybrid",
    tag = "search",
    params(
        ("name" = String, Path, description = "Collection name")
    ),
    request_body = HybridSearchRequest,
    responses(
        (status = 200, description = "Hybrid search results", body = SearchResponse),
        (status = 404, description = "Collection not found", body = crate::types::ErrorResponse),
        (status = 400, description = "Invalid request", body = crate::types::ErrorResponse)
    )
)]
#[allow(clippy::unused_async)]
pub async fn hybrid_search(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Path(name): Path<String>,
    Json(req): Json<HybridSearchRequest>,
) -> impl IntoResponse {
    let collection = match search_preamble(&state, &name, &headers) {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    let start = std::time::Instant::now();

    let expected_dimension = collection.config().dimension;
    if let Err(error) = validate_query_dimension(&state, &name, expected_dimension, &req.vector) {
        return (StatusCode::BAD_REQUEST, Json(error)).into_response();
    }

    let search_result = if let Some(ref filter_json) = req.filter {
        let filter = match parse_filter_or_400(filter_json, &state.onboarding_metrics) {
            Ok(f) => f,
            Err(resp) => return resp,
        };
        collection.hybrid_search_with_filter(
            &req.vector,
            &req.query,
            req.top_k,
            Some(req.vector_weight),
            &filter,
        )
    } else {
        collection.hybrid_search(&req.vector, &req.query, req.top_k, Some(req.vector_weight))
    };

    finish_search_with_cb(&state, &name, start, &collection, search_result)
}

/// Lightweight search returning only IDs and scores (no payload hydration).
///
/// Supports the same search modes as the standard `/search` endpoint:
/// dense, sparse, and hybrid. Honors filter, ef_search, mode, fusion,
/// and all other `SearchRequest` parameters.
#[utoipa::path(
    post,
    path = "/collections/{name}/search/ids",
    tag = "search",
    params(
        ("name" = String, Path, description = "Collection name")
    ),
    request_body = SearchRequest,
    responses(
        (status = 200, description = "IDs-only search results", body = SearchIdsResponse),
        (status = 404, description = "Collection not found", body = crate::types::ErrorResponse),
        (status = 400, description = "Invalid request", body = crate::types::ErrorResponse)
    )
)]
#[allow(clippy::unused_async)]
pub async fn search_ids(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Path(name): Path<String>,
    Json(mut req): Json<SearchRequest>,
) -> impl IntoResponse {
    let start = std::time::Instant::now();

    let collection = match search_preamble(&state, &name, &headers) {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    let search_result = match execute_with_cb(&state, &name, &collection, &mut req) {
        Ok(r) => r,
        Err(resp) => return resp,
    };

    finish_search_ids_with_cb(&state, &name, start, &collection, search_result)
}
