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
    finish_search_with_status, parse_filter_or_400, run_blocking_search,
    run_search_with_optional_timeout, timeout_response, validate_query_dimension,
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
#[allow(clippy::result_large_err)]
pub async fn search(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Path(name): Path<String>,
    Json(req): Json<SearchRequest>,
) -> impl IntoResponse {
    let start = std::time::Instant::now();

    let collection = match search_preamble(&state, &name, &headers) {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    // F-03: honour the per-request `timeout_ms` budget. The synchronous
    // search runs on a blocking worker so the async runtime stays
    // responsive and the timer can actually fire. See
    // `run_search_with_optional_timeout` for the cancellation contract.
    let timeout_ms = req.timeout_ms;
    let state_for_work = Arc::clone(&state);
    let name_for_work = name.clone();
    let collection_for_work = collection.clone();

    let execution = run_search_with_optional_timeout(timeout_ms, move || {
        let mut owned_req = req;
        execute_search_with_cb_owned(
            &state_for_work,
            &name_for_work,
            &collection_for_work,
            &mut owned_req,
        )
    })
    .await;

    let search_result = match execution {
        Ok(Ok(inner)) => inner,
        Ok(Err(resp)) => return resp,
        Err(pipeline::TimeoutElapsed) => {
            // Timeout elapsed: record the circuit-breaker failure and
            // return a 408 with the budget echoed back to the caller.
            collection.guard_rails().circuit_breaker.record_failure();
            let ms = timeout_ms.unwrap_or_default();
            return timeout_response(&name, ms);
        }
    };

    finish_search_with_cb(&state, &name, start, &collection, search_result)
}

/// Owned-request wrapper around [`execute_with_cb`] used by the
/// `run_search_with_optional_timeout` spawn_blocking closure. Having a
/// dedicated function keeps the move-semantics inside the closure
/// explicit and avoids lifetime juggling in the handler body.
#[allow(clippy::result_large_err)]
fn execute_search_with_cb_owned(
    state: &AppState,
    name: &str,
    collection: &VectorCollection,
    req: &mut SearchRequest,
) -> Result<velesdb_core::Result<Vec<velesdb_core::SearchResult>>, axum::response::Response> {
    execute_with_cb(state, name, collection, req)
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
#[allow(clippy::result_large_err)]
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

    // F-01 / F-03 sweep: the BM25 text search is CPU-bound and was
    // previously executed on the async runtime thread. Move it to a
    // blocking worker so concurrent requests do not stall.
    let filter_json = req.filter.clone();
    let query = req.query.clone();
    let top_k = req.top_k;
    let collection_for_work = collection.clone();
    let onboarding_for_work = Arc::clone(&state);

    let work_result = run_blocking_search(move || {
        let filter = match filter_json.as_ref() {
            Some(fj) => match parse_filter_or_400(fj, &onboarding_for_work.onboarding_metrics) {
                Ok(f) => Some(f),
                Err(resp) => return Err(resp),
            },
            None => None,
        };
        Ok(if let Some(f) = filter {
            collection_for_work.text_search_with_filter(&query, top_k, &f)
        } else {
            collection_for_work.text_search(&query, top_k)
        })
    })
    .await;

    let search_result = match work_result {
        Ok(inner) => inner,
        Err(resp) => return resp,
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
#[allow(clippy::result_large_err)]
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

    // F-01 / F-03 sweep: the hybrid BM25 + dense path is CPU-bound
    // (filter parsing, text index lookup, dense HNSW search, fusion).
    // Move the whole closure to a blocking worker so the async
    // runtime stays responsive.
    let collection_for_work = collection.clone();
    let state_for_work = Arc::clone(&state);
    let HybridSearchRequest {
        vector,
        query,
        top_k,
        vector_weight,
        filter,
    } = req;

    let work_result = run_blocking_search(move || {
        let filter = match filter.as_ref() {
            Some(fj) => match parse_filter_or_400(fj, &state_for_work.onboarding_metrics) {
                Ok(f) => Some(f),
                Err(resp) => return Err(resp),
            },
            None => None,
        };
        Ok(if let Some(f) = filter {
            collection_for_work.hybrid_search_with_filter(
                &vector,
                &query,
                top_k,
                Some(vector_weight),
                &f,
            )
        } else {
            collection_for_work.hybrid_search(&vector, &query, top_k, Some(vector_weight))
        })
    })
    .await;

    let search_result = match work_result {
        Ok(inner) => inner,
        Err(resp) => return resp,
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
#[allow(clippy::result_large_err)]
pub async fn search_ids(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Path(name): Path<String>,
    Json(req): Json<SearchRequest>,
) -> impl IntoResponse {
    let start = std::time::Instant::now();

    let collection = match search_preamble(&state, &name, &headers) {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    // F-03: honour the per-request `timeout_ms` budget and run the
    // CPU-bound search on a blocking worker so the async runtime stays
    // responsive. Mirrors the pattern used by `search` so both endpoints
    // share the same timeout semantics for the same `SearchRequest` type.
    let timeout_ms = req.timeout_ms;
    let state_for_work = Arc::clone(&state);
    let name_for_work = name.clone();
    let collection_for_work = collection.clone();

    let execution = run_search_with_optional_timeout(timeout_ms, move || {
        let mut owned_req = req;
        execute_search_with_cb_owned(
            &state_for_work,
            &name_for_work,
            &collection_for_work,
            &mut owned_req,
        )
    })
    .await;

    let search_result = match execution {
        Ok(Ok(inner)) => inner,
        Ok(Err(resp)) => return resp,
        Err(pipeline::TimeoutElapsed) => {
            collection.guard_rails().circuit_breaker.record_failure();
            let ms = timeout_ms.unwrap_or_default();
            return timeout_response(&name, ms);
        }
    };

    finish_search_ids_with_cb(&state, &name, start, &collection, search_result)
}
