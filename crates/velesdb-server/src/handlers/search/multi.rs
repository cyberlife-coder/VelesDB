//! Multi-query search handler: fuse results from multiple query vectors.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use std::sync::Arc;

use crate::types::{ErrorResponse, MultiQuerySearchRequest, SearchResponse};
use crate::AppState;

use super::pipeline::{finish_search_with_cb, parse_filter_or_400, validate_query_dimension};
use crate::handlers::helpers::{apply_pre_check, extract_client_id, get_vector_collection_or_404};

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
#[allow(clippy::unused_async, clippy::too_many_lines)]
pub async fn multi_query_search(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Path(name): Path<String>,
    Json(req): Json<MultiQuerySearchRequest>,
) -> impl IntoResponse {
    use velesdb_core::FusionStrategy;
    state.onboarding_metrics.record_search_request();

    let collection: velesdb_core::collection::VectorCollection =
        match get_vector_collection_or_404(&state, &name) {
            Ok(c) => c,
            Err(resp) => return resp,
        };

    let client_id = extract_client_id(&headers);
    if let Err(resp) = apply_pre_check(collection.guard_rails(), &client_id) {
        return resp;
    }

    let strategy = match req.strategy.to_lowercase().as_str() {
        "average" | "avg" => FusionStrategy::Average,
        "maximum" | "max" => FusionStrategy::Maximum,
        "rrf" => FusionStrategy::RRF { k: req.rrf_k },
        "weighted" => FusionStrategy::Weighted {
            avg_weight: req.avg_weight,
            max_weight: req.max_weight,
            hit_weight: req.hit_weight,
        },
        "relative_score" | "rsf" => FusionStrategy::RelativeScore {
            dense_weight: req.dense_weight,
            sparse_weight: req.sparse_weight,
        },
        _ => {
            return (
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
                .into_response()
        }
    };

    let expected_dimension = collection.config().dimension;
    for (idx, vector) in req.vectors.iter().enumerate() {
        if let Err(error) = validate_query_dimension(&state, &name, expected_dimension, vector) {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!("Invalid query vector at index {idx}: {}", error.error),
                    code: error.code.clone(),
                }),
            )
                .into_response();
        }
    }

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
            Err(resp) => return resp,
        },
        None => None,
    };

    let start = std::time::Instant::now();
    let query_refs: Vec<&[f32]> = req.vectors.iter().map(Vec::as_slice).collect();

    let search_result =
        collection.multi_query_search(&query_refs, req.top_k, strategy, filter.as_ref());

    finish_search_with_cb(&state, &name, start, &collection, search_result)
}
