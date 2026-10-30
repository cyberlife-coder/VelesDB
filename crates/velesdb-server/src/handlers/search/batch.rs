//! Batch search handler: multiple vector queries in a single request.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use std::sync::Arc;

use crate::types::{
    parse_search_mode, validate_ef_search, BatchSearchRequest, BatchSearchResponse, ErrorResponse,
    SearchResponse,
};
use crate::AppState;

use super::outcome::{
    build_search_response, read_refused, record_circuit_breaker, search_error_response,
};
use super::pipeline::validate_query_dimension;
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
        (status = 429, description = "The client's rate limit", body = ErrorResponse),
        (status = 500, description = "An engine fault (storage, I/O, internal)", body = ErrorResponse),
        (status = 503, description = "The circuit breaker is open, or a guard rail the engine could not carry stopped the search", body = ErrorResponse),
        (status = 404, description = "Collection not found", body = ErrorResponse),
        (status = 400, description = "Invalid request", body = ErrorResponse),
        (
            status = 422,
            description = "A field of the wrong type in an entry, such as a negative ef_search",
            body = String,
            content_type = "text/plain"
        )
    )
)]
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

    // Record query type only after confirming the collection exists, so
    // 404s do not inflate queries_total or vector_queries.
    state.operational_metrics.record_vector_query();

    let client_id = extract_client_id(&headers);
    if let Err(resp) = apply_pre_check(collection.guard_rails(), &client_id) {
        state.operational_metrics.inc_rate_limited();
        return resp;
    }

    if let Err(resp) = validate_batch_queries(&state, &name, &collection, &req) {
        state.operational_metrics.inc_errors();
        return resp;
    }

    let filters = match parse_batch_filters(&state, &req) {
        Ok(f) => f,
        Err(resp) => {
            state.operational_metrics.inc_errors();
            return resp;
        }
    };

    // Gate the read (CORE-2) before dispatching the off-thread batch. A denied
    // decision — or a scope narrowing, which batch has no per-query channel to
    // apply — refuses the batch (fail closed).
    match state.db.authorize_read(
        &name,
        velesdb_core::observer::QueryOperationKind::VectorSearch,
        None,
        None,
    ) {
        Ok(None) => {}
        Ok(Some(_)) => {
            state.operational_metrics.inc_errors();
            return search_error_response(&velesdb_core::Error::Config(
                "scope narrowing is not supported for batch search".to_string(),
            ));
        }
        Err(refusal) => {
            state.operational_metrics.inc_errors();
            return read_refused(&refusal);
        }
    }

    // F-01 sweep: batch search is CPU-bound (one HNSW pass per query) and must
    // not run on the async runtime thread — a large batch would otherwise block
    // a Tokio worker and starve concurrent requests. Move it to a blocking
    // worker like the other search endpoints. `spawn_blocking` needs a 'static
    // closure, so owned query vectors + filters are moved in and the `&[f32]`
    // views are rebuilt inside.
    let collection_for_work = collection.clone();
    let query_vectors: Vec<Vec<f32>> = req.searches.iter().map(|s| s.vector.clone()).collect();
    let max_top_k = req.searches.iter().map(|s| s.top_k).max().unwrap_or(10);

    let batch_result = match tokio::task::spawn_blocking(move || {
        let queries: Vec<&[f32]> = query_vectors.iter().map(Vec::as_slice).collect();
        run_batch_search(&collection_for_work, &queries, max_top_k, &filters)
    })
    .await
    {
        Ok(inner) => inner,
        Err(_) => {
            state.operational_metrics.inc_errors();
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Batch search worker failed".to_string(),
                    code: None,
                }),
            )
                .into_response();
        }
    };
    record_circuit_breaker(&collection, &batch_result);

    let all_results = match batch_result {
        Ok(batch_results) => build_batch_responses(&state, batch_results, &req),
        Err(e) => {
            state.operational_metrics.inc_errors();
            return search_error_response(&e);
        }
    };

    finish_batch_search(&state, &name, start, all_results)
}

/// Dispatch a batch search to the throughput-optimized parallel kernel when no
/// query carries a filter, falling back to the per-query filtered kernel
/// otherwise. Both paths share the same HNSW traversal, so results are
/// identical for the unfiltered case — only the rayon parallelism differs.
fn run_batch_search(
    collection: &velesdb_core::collection::VectorCollection,
    queries: &[&[f32]],
    max_top_k: usize,
    filters: &[Option<velesdb_core::Filter>],
) -> velesdb_core::Result<Vec<Vec<velesdb_core::SearchResult>>> {
    if filters.iter().all(Option::is_none) {
        collection.search_batch_parallel(queries, max_top_k)
    } else {
        collection.search_batch_with_filters(queries, max_top_k, filters)
    }
}

/// Record timing metrics and build the final batch response envelope.
fn finish_batch_search(
    state: &AppState,
    name: &str,
    start: std::time::Instant,
    results: Vec<SearchResponse>,
) -> axum::response::Response {
    let elapsed = start.elapsed();
    let timing_ms = elapsed.as_secs_f64() * 1000.0;
    notify_query_timing(state, name, start);
    state
        .query_duration_histogram
        .observe(elapsed.as_secs_f64());

    Json(BatchSearchResponse { results, timing_ms }).into_response()
}

/// Validate every query of a batch request: its vector matches the collection
/// dimension, and its search mode and `ef_search`, when given, are ones the
/// parser reads and the documented range accepts. The first bad query gets a
/// 400 naming its index. The batch applies neither override, but a typo or an
/// out-of-range value must not pass silently there either (#2267, #2274).
#[allow(clippy::result_large_err)]
fn validate_batch_queries(
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
            return Err(invalid_batch_query(idx, &error.error, error.code.clone()));
        }
        if let Some(Err(error)) = search.mode.as_deref().map(parse_search_mode) {
            return Err(invalid_batch_query(idx, &error, None));
        }
        if let Some(Err(error)) = search.ef_search.map(validate_ef_search) {
            return Err(invalid_batch_query(idx, &error, None));
        }
    }
    Ok(())
}

/// The 400 answering the batch query at `idx`.
fn invalid_batch_query(idx: usize, error: &str, code: Option<String>) -> axum::response::Response {
    (
        StatusCode::BAD_REQUEST,
        Json(ErrorResponse {
            error: format!("Invalid query at index {idx}: {error}"),
            code,
        }),
    )
        .into_response()
}

/// Parse filters from each search in a batch request.
#[allow(clippy::result_large_err)]
fn parse_batch_filters(
    state: &AppState,
    req: &BatchSearchRequest,
) -> Result<Vec<Option<velesdb_core::Filter>>, axum::response::Response> {
    req.searches
        .iter()
        .enumerate()
        .map(|(idx, search)| {
            let Some(filter_json) = &search.filter else {
                return Ok(None);
            };
            serde_json::from_value(filter_json.clone())
                .map(Some)
                .map_err(|e| {
                    state.onboarding_metrics.record_filter_parse_error();
                    (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: format!(
                                "Invalid filter at index {idx}: {e}. Hint: validate filter syntax and start with a broader query before reintroducing strict filters."
                            ),
                            code: None,
                        }),
                    )
                        .into_response()
                })
        })
        .collect()
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
        .map(|(results, search)| {
            let truncated: Vec<_> = results.into_iter().take(search.top_k).collect();
            build_search_response(truncated)
        })
        .collect()
}
