//! Multi-query search handler: fuse results from multiple query vectors.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use std::sync::Arc;

use crate::types::{
    ErrorResponse, MultiQuerySearchRequestWithWeights, SearchIdsResponse, SearchResponse,
};
use crate::AppState;

use super::outcome::{finish_search_ids_with_cb, finish_search_with_cb, read_refused};
use super::pipeline::{id_score_results, parse_filter_or_400, validate_query_dimension};
use super::workers::run_blocking_search;
use crate::handlers::helpers::{
    apply_pre_check, extract_client_id, get_vector_collection_or_404, scope_unsupported,
};

/// Build the `FusionStrategy` a multi-query request asks for, or the message
/// explaining why it cannot be built.
///
/// Split from [`parse_fusion_strategy`] so the decision is testable without
/// an `AppState`: everything here is a pure function of the request body,
/// and the caller owns the metrics and the HTTP shell.
///
/// The name, its aliases and the weight rules are core's
/// ([`velesdb_core::FusionStrategy::from_name`]), and its message is reported
/// verbatim, so the server never carries a second, driftable copy of either.
/// The weighted strategies therefore go through their validating
/// constructors: struct literals accepted any `f32` the JSON body carried, so
/// a negative weight, a set that did not sum to 1.0, or a non-finite value
/// reached the fusion kernel and came back as a silently wrong ranking
/// instead of an error (#2095).
pub(super) fn build_fusion_strategy(
    req: &MultiQuerySearchRequestWithWeights,
) -> Result<velesdb_core::FusionStrategy, String> {
    let reads_branch_weights = ["weighted_rrf", "relative_score", "rsf"]
        .iter()
        .any(|name| req.base.strategy.eq_ignore_ascii_case(name));
    if reads_branch_weights
        && req
            .branch_weights
            .as_ref()
            .is_some_and(|w| !w.is_empty() && w.len() != req.base.vectors.len())
    {
        return Err(format!(
            "{} requires one weight per query vector: {} weights for {} vectors",
            req.base.strategy,
            req.branch_weights.as_ref().map_or(0, Vec::len),
            req.base.vectors.len(),
        ));
    }
    let params = velesdb_core::FusionParams::default()
        .with_rrf_k(Some(req.base.rrf_k))
        .with_avg_weight(Some(req.base.avg_weight))
        .with_max_weight(Some(req.base.max_weight))
        .with_hit_weight(Some(req.base.hit_weight))
        .with_dense_weight(Some(req.base.dense_weight))
        .with_sparse_weight(Some(req.base.sparse_weight));
    velesdb_core::FusionStrategy::from_name_with_weights(
        &req.base.strategy,
        params,
        req.branch_weights.as_deref(),
    )
    .map_err(|e| e.to_string())
}

/// Parse the fusion strategy the request asks for, returning a 400 response
/// (and bumping the error counter) for an unknown strategy or for weights the
/// strategy refuses.
#[allow(clippy::result_large_err)]
fn parse_fusion_strategy(
    req: &MultiQuerySearchRequestWithWeights,
    state: &AppState,
) -> Result<velesdb_core::FusionStrategy, axum::response::Response> {
    build_fusion_strategy(req).map_err(|error| {
        state.operational_metrics.inc_errors();
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse { error, code: None }),
        )
            .into_response()
    })
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
    req: &MultiQuerySearchRequestWithWeights,
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
    validate_query_vectors(state, name, expected_dimension, &req.base.vectors)?;

    Ok((collection, strategy))
}

/// Multi-query search with fusion strategies.
#[utoipa::path(
    post,
    path = "/collections/{name}/search/multi",
    tag = "search",
    params(("name" = String, Path, description = "Collection name")),
    request_body = MultiQuerySearchRequestWithWeights,
    responses(
        (status = 200, description = "Multi-query search results", body = SearchResponse),
        (status = 400, description = "Invalid request", body = ErrorResponse),
        (status = 429, description = "The client's rate limit", body = ErrorResponse),
        (status = 503, description = "The circuit breaker is open, or a guard rail the engine could not carry stopped the search", body = ErrorResponse),
        (status = 404, description = "Collection not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse),
        (status = 422, response = crate::types::MalformedBody)
    )
)]
#[allow(clippy::result_large_err)]
pub async fn multi_query_search(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Path(name): Path<String>,
    Json(req): Json<MultiQuerySearchRequestWithWeights>,
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
    let filter = match req.base.filter.as_ref() {
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
    //
    // The read goes through the control-plane gate (CORE-2) inside the worker,
    // which also reports it to the observer. A denial — or a scope narrowing,
    // which the fusion kernel has no channel to apply — refuses the query
    // (fail closed).
    let collection_for_work = collection.clone();
    let state_for_work = Arc::clone(&state);
    let name_for_work = name.clone();
    let vectors = req.base.vectors;
    let top_k = req.base.top_k;

    let work_result = run_blocking_search(move || {
        let query_refs: Vec<&[f32]> = vectors.iter().map(Vec::as_slice).collect();
        run_unscoped_vector_read(&state_for_work, &name_for_work, || {
            collection_for_work.multi_query_search(&query_refs, top_k, strategy, filter.as_ref())
        })
        .map_err(|refusal| read_refused(&refusal))
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
    request_body = MultiQuerySearchRequestWithWeights,
    responses(
        (status = 200, description = "Multi-query ids-only results", body = SearchIdsResponse),
        (status = 429, description = "The client's rate limit", body = ErrorResponse),
        (status = 500, description = "An engine fault (storage, I/O, internal)", body = ErrorResponse),
        (status = 503, description = "The circuit breaker is open, or a guard rail the engine could not carry stopped the search", body = ErrorResponse),
        (status = 400, description = "Invalid request", body = ErrorResponse),
        (status = 404, description = "Collection not found", body = ErrorResponse),
        (status = 422, response = crate::types::MalformedBody)
    )
)]
#[allow(clippy::result_large_err)]
pub async fn multi_query_search_ids(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Path(name): Path<String>,
    Json(req): Json<MultiQuerySearchRequestWithWeights>,
) -> impl IntoResponse {
    let (collection, strategy) = match prepare_multi_query(&state, &headers, &name, &req) {
        Ok(v) => v,
        Err(resp) => return resp,
    };

    // The ids-only fusion kernel has no filter parameter. Reject filters
    // explicitly rather than silently returning unfiltered results.
    if req.base.filter.is_some() {
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

    // Gated (CORE-2) and reported inside the worker, as `/search/multi`.
    let start = std::time::Instant::now();
    let collection_for_work = collection.clone();
    let state_for_work = Arc::clone(&state);
    let name_for_work = name.clone();
    let vectors = req.base.vectors;
    let top_k = req.base.top_k;

    let work_result = run_blocking_search(move || {
        let query_refs: Vec<&[f32]> = vectors.iter().map(Vec::as_slice).collect();
        run_unscoped_vector_read(&state_for_work, &name_for_work, || {
            collection_for_work
                .multi_query_search_ids(&query_refs, top_k, strategy)
                .map(id_score_results)
        })
        .map_err(|refusal| read_refused(&refusal))
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

/// Runs a multi-query fusion through the read gate, which reports it. The
/// outer error is the gate's refusal; a scope narrowing — which the fusion
/// kernels have no channel to apply — fails closed as the inner one.
fn run_unscoped_vector_read<T>(
    state: &AppState,
    name: &str,
    read: impl FnOnce() -> velesdb_core::Result<T>,
) -> velesdb_core::Result<velesdb_core::Result<T>> {
    state.db.run_gated_read(
        name,
        velesdb_core::observer::QueryOperationKind::VectorSearch,
        None,
        None,
        |scope| match scope {
            None => read(),
            Some(_) => Err(scope_unsupported("multi-query search")),
        },
    )
}
