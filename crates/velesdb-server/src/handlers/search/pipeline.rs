//! Search pipeline helpers: validation, sparse resolution, fusion parsing,
//! and shared result handling.

use axum::{http::StatusCode, response::IntoResponse, Json};
use velesdb_core::collection::VectorCollection;
use velesdb_core::index::sparse::DEFAULT_SPARSE_INDEX_NAME;

use crate::types::{
    mode_to_ef_search, ErrorResponse, IdScoreResult, SearchIdsResponse, SearchRequest,
    SearchResponse, SearchResultResponse,
};
use crate::AppState;

/// Convert a `Vec<SearchResult>` into a `SearchResponse`.
pub(crate) fn build_search_response(results: Vec<velesdb_core::SearchResult>) -> SearchResponse {
    SearchResponse {
        results: results
            .into_iter()
            .map(|r| SearchResultResponse {
                id: r.point.id,
                score: r.score,
                payload: r.point.payload,
            })
            .collect(),
    }
}

/// Parse a JSON value into a `Filter`, returning a 400 response on failure.
#[allow(clippy::result_large_err)]
pub(crate) fn parse_filter_or_400(
    filter_json: &serde_json::Value,
    onboarding_metrics: &crate::OnboardingMetrics,
) -> Result<velesdb_core::Filter, axum::response::Response> {
    serde_json::from_value(filter_json.clone()).map_err(|e| {
        onboarding_metrics.record_filter_parse_error();
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Invalid filter: {e}"),
            }),
        )
            .into_response()
    })
}

pub(crate) fn dimension_mismatch_error(
    collection_name: &str,
    expected: usize,
    actual: usize,
) -> ErrorResponse {
    ErrorResponse {
        error: format!(
            "Vector dimension mismatch for collection '{collection_name}': expected {expected}, got {actual}. Hint: use embeddings with the same dimension as the collection or create a new collection with the target dimension."
        ),
    }
}

pub(crate) fn validate_query_dimension(
    state: &AppState,
    collection_name: &str,
    expected: usize,
    query_vector: &[f32],
) -> Result<(), ErrorResponse> {
    let actual = query_vector.len();
    if actual == expected {
        return Ok(());
    }
    state.onboarding_metrics.record_dimension_mismatch();
    tracing::warn!(
        collection = %collection_name,
        expected_dimension = expected,
        actual_dimension = actual,
        "Search rejected due to vector dimension mismatch"
    );
    Err(dimension_mismatch_error(collection_name, expected, actual))
}

pub(crate) fn actionable_search_error(error: &dyn std::fmt::Display) -> ErrorResponse {
    let base_error = error.to_string();
    let lower = base_error.to_lowercase();
    let hint = if lower.contains("dimension") {
        " Hint: check that query vector dimension matches collection dimension."
    } else if lower.contains("filter") {
        " Hint: validate filter syntax and start with a broader query before reintroducing strict filters."
    } else {
        " Hint: if you get empty results, retry without strict filters/thresholds, then tighten progressively."
    };

    ErrorResponse {
        error: format!("{base_error}{hint}"),
    }
}

/// Resolves sparse input from a `SearchRequest`, validating ambiguity rules.
///
/// Returns `Ok(Some(SparseVector))` for valid sparse input, `Ok(None)` if no
/// sparse input was provided, or `Err(Response)` on validation failure.
#[allow(clippy::result_large_err)]
pub(crate) fn resolve_sparse_input(
    req: &mut SearchRequest,
) -> Result<Option<velesdb_core::index::sparse::SparseVector>, axum::response::Response> {
    let raw = if req.sparse_vector.is_some() {
        req.sparse_vector.take()
    } else if let Some(ref mut m) = req.sparse_vectors {
        if m.len() > 1 && req.sparse_index.is_none() {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!(
                        "Ambiguous sparse query: {} named sparse vectors supplied but \
                         'sparse_index' was not specified. \
                         Provide 'sparse_index' to select which one to use, \
                         or supply a single 'sparse_vector'.",
                        m.len()
                    ),
                }),
            )
                .into_response());
        }
        if let Some(ref idx_name) = req.sparse_index {
            m.remove(idx_name.as_str())
        } else {
            m.pop_first().map(|(_, v)| v)
        }
    } else {
        None
    };

    match raw {
        Some(sv_input) => match sv_input.into_sparse_vector() {
            Ok(sv) => Ok(Some(sv)),
            Err(e) => {
                Err((StatusCode::BAD_REQUEST, Json(ErrorResponse { error: e })).into_response())
            }
        },
        None => Ok(None),
    }
}

/// Parses fusion configuration into a core `FusionStrategy`.
///
/// Defaults to RRF k=60 when no fusion config is provided.
#[allow(clippy::result_large_err)]
pub(crate) fn parse_fusion_strategy(
    fusion: Option<&crate::types::FusionRequest>,
) -> Result<velesdb_core::FusionStrategy, axum::response::Response> {
    let f = match fusion {
        None => return Ok(velesdb_core::FusionStrategy::rrf_default()),
        Some(f) => f,
    };
    match f.strategy.to_lowercase().as_str() {
        "rrf" => Ok(velesdb_core::FusionStrategy::RRF {
            k: f.k.unwrap_or(60),
        }),
        "rsf" | "relative_score" => {
            let (dw, sw) = match (f.dense_w, f.sparse_w) {
                (Some(d), Some(s)) => (d, s),
                (Some(d), None) => (d, 1.0 - d),
                (None, Some(s)) => (1.0 - s, s),
                (None, None) => (0.5, 0.5),
            };
            velesdb_core::FusionStrategy::relative_score(dw, sw).map_err(|e| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: format!("Invalid RSF fusion weights: {e}"),
                    }),
                )
                    .into_response()
            })
        }
        other => Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!(
                    "Invalid fusion strategy: '{other}'. \
                     Valid values: 'rrf', 'rsf' (alias: 'relative_score')"
                ),
            }),
        )
            .into_response()),
    }
}

/// Executes the dense-only search path, honoring filter, ef_search, and mode.
#[allow(clippy::result_large_err)]
pub(crate) fn execute_dense_search(
    state: &AppState,
    name: &str,
    collection: &VectorCollection,
    req: &SearchRequest,
) -> Result<velesdb_core::Result<Vec<velesdb_core::SearchResult>>, axum::response::Response> {
    let expected_dimension = collection.config().dimension;
    if let Err(error) = validate_query_dimension(state, name, expected_dimension, &req.vector) {
        return Err((StatusCode::BAD_REQUEST, Json(error)).into_response());
    }

    let effective_ef = req
        .ef_search
        .or_else(|| req.mode.as_ref().and_then(|m| mode_to_ef_search(m)));

    let result = if let Some(ref filter_json) = req.filter {
        let filter = parse_filter_or_400(filter_json, &state.onboarding_metrics)?;
        collection.search_with_filter(&req.vector, req.top_k, &filter)
    } else if let Some(ef) = effective_ef {
        collection.search_with_ef(&req.vector, req.top_k, ef)
    } else {
        collection.search(&req.vector, req.top_k)
    };
    Ok(result)
}

/// Runs the full search pipeline (dense, sparse, or hybrid) based on
/// `SearchRequest` fields. Returns search results or an error response.
#[allow(clippy::result_large_err)]
pub(crate) fn execute_search_request(
    state: &AppState,
    name: &str,
    collection: &VectorCollection,
    req: &mut SearchRequest,
) -> Result<velesdb_core::Result<Vec<velesdb_core::SearchResult>>, axum::response::Response> {
    let sparse_vec = resolve_sparse_input(req)?;
    let has_dense = !req.vector.is_empty();
    let has_sparse = sparse_vec.is_some();

    if !has_dense && !has_sparse {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Either 'vector' or 'sparse_vector' must be provided".to_string(),
            }),
        )
            .into_response());
    }

    let index_name = req
        .sparse_index
        .as_deref()
        .unwrap_or(DEFAULT_SPARSE_INDEX_NAME);

    // Hybrid: both dense and sparse
    if has_dense {
        if let Some(sparse_query) = sparse_vec {
            return execute_hybrid_sparse(state, name, collection, req, &sparse_query, index_name);
        }
        // Dense-only
        return execute_dense_search(state, name, collection, req);
    }

    // Sparse-only
    if let Some(sparse_query) = sparse_vec {
        return Ok(collection.sparse_search(&sparse_query, req.top_k, index_name));
    }

    // Dense-only (fallback — should not reach here given earlier validation)
    execute_dense_search(state, name, collection, req)
}

/// Hybrid dense+sparse search path with dimension validation and fusion.
#[allow(clippy::result_large_err)]
fn execute_hybrid_sparse(
    state: &AppState,
    name: &str,
    collection: &VectorCollection,
    req: &SearchRequest,
    sparse_query: &velesdb_core::index::sparse::SparseVector,
    index_name: &str,
) -> Result<velesdb_core::Result<Vec<velesdb_core::SearchResult>>, axum::response::Response> {
    let expected_dimension = collection.config().dimension;
    if let Err(error) = validate_query_dimension(state, name, expected_dimension, &req.vector) {
        return Err((StatusCode::BAD_REQUEST, Json(error)).into_response());
    }
    let strategy = parse_fusion_strategy(req.fusion.as_ref())?;
    Ok(
        collection.hybrid_sparse_search(
            &req.vector,
            sparse_query,
            req.top_k,
            index_name,
            &strategy,
        ),
    )
}

/// Record empty-results diagnostic and notify the query timing subsystem.
fn record_search_metrics(state: &AppState, name: &str, start: std::time::Instant, is_empty: bool) {
    if is_empty {
        state.onboarding_metrics.record_empty_search_results();
    }
    let duration_us = start.elapsed().as_micros();
    #[allow(clippy::cast_possible_truncation)]
    // Reason: value is clamped to u64::MAX above, so the truncation is lossless.
    state
        .db
        .notify_query(name, duration_us.min(u128::from(u64::MAX)) as u64);
}

/// Core search result handler: records metrics, delegates success to `on_ok`,
/// returns actionable error response on failure.
fn finish_search_core(
    state: &AppState,
    name: &str,
    start: std::time::Instant,
    error_status: StatusCode,
    search_result: velesdb_core::Result<Vec<velesdb_core::SearchResult>>,
    on_ok: impl FnOnce(Vec<velesdb_core::SearchResult>) -> axum::response::Response,
) -> axum::response::Response {
    match search_result {
        Ok(results) => {
            record_search_metrics(state, name, start, results.is_empty());
            on_ok(results)
        }
        Err(e) => (error_status, Json(actionable_search_error(&e))).into_response(),
    }
}

/// Shared result-handling for all search modes.
pub(crate) fn finish_search(
    state: &AppState,
    name: &str,
    start: std::time::Instant,
    search_result: velesdb_core::Result<Vec<velesdb_core::SearchResult>>,
) -> axum::response::Response {
    finish_search_core(
        state,
        name,
        start,
        StatusCode::BAD_REQUEST,
        search_result,
        |results| Json(build_search_response(results)).into_response(),
    )
}

/// Maps search results to IDs+scores response with timing metrics.
pub(crate) fn finish_search_ids(
    state: &AppState,
    name: &str,
    start: std::time::Instant,
    search_result: velesdb_core::Result<Vec<velesdb_core::SearchResult>>,
) -> axum::response::Response {
    finish_search_core(
        state,
        name,
        start,
        StatusCode::BAD_REQUEST,
        search_result,
        |results| {
            let response = SearchIdsResponse {
                results: results
                    .into_iter()
                    .map(|r| IdScoreResult {
                        id: r.point.id,
                        score: r.score,
                    })
                    .collect(),
            };
            Json(response).into_response()
        },
    )
}

/// Record circuit-breaker outcome (success/failure) based on a search result.
pub(crate) fn record_circuit_breaker<T>(
    collection: &VectorCollection,
    result: &velesdb_core::Result<T>,
) {
    if result.is_ok() {
        collection.guard_rails().circuit_breaker.record_success();
    } else {
        collection.guard_rails().circuit_breaker.record_failure();
    }
}

/// Handles `Ok`/`Err` from a core search call: records circuit-breaker
/// outcome and delegates to [`finish_search`] for metrics + response.
pub(crate) fn finish_search_with_cb(
    state: &AppState,
    name: &str,
    start: std::time::Instant,
    collection: &VectorCollection,
    search_result: velesdb_core::Result<Vec<velesdb_core::SearchResult>>,
) -> axum::response::Response {
    record_circuit_breaker(collection, &search_result);
    finish_search(state, name, start, search_result)
}

/// Handles `Ok`/`Err` from a core search call: records circuit-breaker
/// outcome and delegates to [`finish_search_ids`] for metrics + response.
pub(crate) fn finish_search_ids_with_cb(
    state: &AppState,
    name: &str,
    start: std::time::Instant,
    collection: &VectorCollection,
    search_result: velesdb_core::Result<Vec<velesdb_core::SearchResult>>,
) -> axum::response::Response {
    record_circuit_breaker(collection, &search_result);
    finish_search_ids(state, name, start, search_result)
}

/// Variant of [`finish_search_with_cb`] that uses a custom error status code
/// instead of the default 400 used by [`finish_search`].
pub(crate) fn finish_search_with_status(
    state: &AppState,
    name: &str,
    start: std::time::Instant,
    collection: &VectorCollection,
    error_status: StatusCode,
    search_result: velesdb_core::Result<Vec<velesdb_core::SearchResult>>,
) -> axum::response::Response {
    record_circuit_breaker(collection, &search_result);
    finish_search_core(state, name, start, error_status, search_result, |results| {
        Json(build_search_response(results)).into_response()
    })
}
