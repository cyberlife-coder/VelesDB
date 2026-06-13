//! Multi-query search handler: fuse results from multiple query vectors.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use std::sync::Arc;

use crate::types::{ErrorResponse, MultiQuerySearchRequest, SearchIdsResponse, SearchResponse};
use crate::AppState;

use super::pipeline::{
    finish_search_ids_with_cb, finish_search_with_cb, id_score_results, parse_filter_or_400,
    validate_query_dimension,
};
use super::workers::run_blocking_search;
use crate::handlers::helpers::{apply_pre_check, extract_client_id, get_vector_collection_or_404};

/// Parse the fusion strategy name into a `FusionStrategy`, returning a 400
/// response (and bumping the error counter) for an unknown strategy.
#[allow(clippy::result_large_err)]
fn parse_fusion_strategy(
    req: &MultiQuerySearchRequest,
    state: &AppState,
) -> Result<velesdb_core::FusionStrategy, axum::response::Response> {
    use velesdb_core::FusionStrategy;
    match req.strategy.to_lowercase().as_str() {
        "average" | "avg" => Ok(FusionStrategy::Average),
        "maximum" | "max" => Ok(FusionStrategy::Maximum),
        "rrf" => Ok(FusionStrategy::RRF { k: req.rrf_k }),
        "weighted" => Ok(FusionStrategy::Weighted {
            avg_weight: req.avg_weight,
            max_weight: req.max_weight,
            hit_weight: req.hit_weight,
        }),
        "relative_score" | "rsf" => Ok(FusionStrategy::RelativeScore {
            dense_weight: req.dense_weight,
            sparse_weight: req.sparse_weight,
        }),
        _ => {
            state.operational_metrics.inc_errors();
            Err((
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!(
                        "Invalid strategy: {}. Valid: average, maximum, rrf, weighted, \
                         relative_score",
                        req.strategy
                    ),
                    code: None,
                }),
            )
                .into_response())
        }
    }
}

/// Validate every query vector's dimension, returning a 400 on the first
/// mismatch (with the offending index in the message).
#[allow(clippy::result_large_err)]
fn validate_query_vectors(
    state: &AppState,
    name: &str,
    expected_dimension: usize,
    vectors: &[Vec<f32>],
) -> Result<(), axum::response::Response> {
    for (idx, vector) in vectors.iter().enumerate() {
        if let Err(error) = validate_query_dimension(state, name, expected_dimension, vector) {
            state.operational_metrics.inc_errors();
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!("Invalid query vector at index {idx}: {}", error.error),
                    code: error.code.clone(),
                }),
            )
                .into_response());
        }
    }
    Ok(())
}

/// Shared preamble for the multi-query handlers: records metrics, resolves the
/// collection, enforces guard rails, parses the fusion strategy, and validates
/// every query vector's dimension. Returns the collection and strategy on
/// success, or the appropriate error response.
#[allow(clippy::result_large_err)]
fn prepare_multi_query(
    state: &AppState,
    headers: &axum::http::HeaderMap,
    name: &str,
    req: &MultiQuerySearchRequest,
) -> Result<
    (
        velesdb_core::collection::VectorCollection,
        velesdb_core::FusionStrategy,
    ),
    axum::response::Response,
> {
    state.onboarding_metrics.record_search_request();

    let collection = get_vector_collection_or_404(state, name)?;

    // Record query type only after confirming the collection exists, so
    // 404s do not inflate queries_total or vector_queries.
    state.operational_metrics.record_vector_query();

    let client_id = extract_client_id(headers);
    if let Err(resp) = apply_pre_check(collection.guard_rails(), &client_id) {
        state.operational_metrics.inc_rate_limited();
        return Err(resp);
    }

    let strategy = parse_fusion_strategy(req, state)?;

    let expected_dimension = collection.config().dimension;
    validate_query_vectors(state, name, expected_dimension, &req.vectors)?;

    Ok((collection, strategy))
}

/// Multi-query search with fusion strategies.
#[utoipa::path(
    post,
    path = "/collections/{name}/search/multi",
    tag = "search",
    params(("name" = String, Path, description = "Collection name")),
    request_body = MultiQuerySearchRequest,
    responses(
        (status = 200, description = "Multi-query search results", body = SearchResponse),
        (status = 404, description = "Collection not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    )
)]
#[allow(clippy::result_large_err)]
pub async fn multi_query_search(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Path(name): Path<String>,
    Json(req): Json<MultiQuerySearchRequest>,
) -> impl IntoResponse {
    let (collection, strategy) = match prepare_multi_query(&state, &headers, &name, &req) {
        Ok(v) => v,
        Err(resp) => return resp,
    };

    // Parse the optional metadata filter. We need to materialise the
    // `Filter` before starting the stopwatch so that a malformed filter
    // yields a 400 response instead of a misleading 200 with unfiltered
    // results. Regression guard: see
    // `test_multi_query_search_with_filter_excludes_nonmatching_points`
    // and `test_multi_query_search_with_invalid_filter_returns_400`
    // (F-04).
    let filter = match req.filter.as_ref() {
        Some(filter_json) => match parse_filter_or_400(filter_json, &state.onboarding_metrics) {
            Ok(f) => Some(f),
            Err(resp) => {
                state.operational_metrics.inc_errors();
                return resp;
            }
        },
        None => None,
    };

    let start = std::time::Instant::now();

    // F-01 sweep: multi-vector fusion is CPU-bound (multiple HNSW
    // passes plus a fusion step) and was previously executed on the
    // async runtime thread. Move it to a blocking worker so concurrent
    // requests stay responsive. We move `vectors` (owned
    // `Vec<Vec<f32>>`) into the closure and rebuild the `&[f32]` slice
    // view inside, because `spawn_blocking` requires a 'static closure
    // and borrowed slice references cannot cross the boundary.
    let collection_for_work = collection.clone();
    let vectors = req.vectors;
    let top_k = req.top_k;

    let work_result = run_blocking_search(move || {
        let query_refs: Vec<&[f32]> = vectors.iter().map(Vec::as_slice).collect();
        Ok(collection_for_work.multi_query_search(&query_refs, top_k, strategy, filter.as_ref()))
    })
    .await;

    let search_result = match work_result {
        Ok(inner) => inner,
        Err(resp) => {
            state.operational_metrics.inc_errors();
            return resp;
        }
    };

    finish_search_with_cb(&state, &name, start, &collection, search_result)
}

/// Multi-query fusion search returning only ids and scores (no payloads).
///
/// Faster than `/search/multi` when payloads are not needed: the core
/// `multi_query_search_ids` kernel skips payload hydration. Metadata filters
/// are not supported here — use `/search/multi` for filtered fusion.
#[utoipa::path(
    post,
    path = "/collections/{name}/search/multi/ids",
    tag = "search",
    params(("name" = String, Path, description = "Collection name")),
    request_body = MultiQuerySearchRequest,
    responses(
        (status = 200, description = "Multi-query ids-only results", body = SearchIdsResponse),
        (status = 400, description = "Invalid request", body = ErrorResponse),
        (status = 404, description = "Collection not found", body = ErrorResponse)
    )
)]
#[allow(clippy::result_large_err)]
pub async fn multi_query_search_ids(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Path(name): Path<String>,
    Json(req): Json<MultiQuerySearchRequest>,
) -> impl IntoResponse {
    let (collection, strategy) = match prepare_multi_query(&state, &headers, &name, &req) {
        Ok(v) => v,
        Err(resp) => return resp,
    };

    // The ids-only fusion kernel has no filter parameter. Reject filters
    // explicitly rather than silently returning unfiltered results.
    if req.filter.is_some() {
        state.operational_metrics.inc_errors();
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Metadata filters are not supported on /search/multi/ids; \
                        use /search/multi for filtered multi-query search."
                    .to_string(),
                code: None,
            }),
        )
            .into_response();
    }

    let start = std::time::Instant::now();
    let collection_for_work = collection.clone();
    let vectors = req.vectors;
    let top_k = req.top_k;

    let work_result = run_blocking_search(move || {
        let query_refs: Vec<&[f32]> = vectors.iter().map(Vec::as_slice).collect();
        Ok(collection_for_work
            .multi_query_search_ids(&query_refs, top_k, strategy)
            .map(id_score_results))
    })
    .await;

    let search_result = match work_result {
        Ok(inner) => inner,
        Err(resp) => {
            state.operational_metrics.inc_errors();
            return resp;
        }
    };

    finish_search_ids_with_cb(&state, &name, start, &collection, search_result)
}
