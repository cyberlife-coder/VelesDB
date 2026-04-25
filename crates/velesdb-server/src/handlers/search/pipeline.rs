//! Search pipeline helpers: validation, sparse resolution, fusion parsing,
//! and shared result handling.

use axum::{http::StatusCode, response::IntoResponse, Json};
use velesdb_core::collection::VectorCollection;
use velesdb_core::index::sparse::DEFAULT_SPARSE_INDEX_NAME;

use crate::types::{
    mode_to_search_quality, ErrorResponse, IdScoreResult, SearchIdsResponse, SearchRequest,
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
    velesdb_core::Filter::from_json_value(filter_json.clone()).map_err(|error| {
        onboarding_metrics.record_filter_parse_error();
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse { error, code: None }),
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
        code: Some("VELES-004".to_string()),
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

pub(crate) fn actionable_search_error(error: &velesdb_core::Error) -> ErrorResponse {
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
        code: Some(error.code().to_string()),
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
                    code: None,
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
            Err(e) => Err((
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: e,
                    code: None,
                }),
            )
                .into_response()),
        },
        None => Ok(None),
    }
}

/// Parses fusion configuration into a core `FusionStrategy`.
///
/// Defaults to RRF k=60 when no fusion config is provided.
///
/// Supported strategy strings (case-insensitive):
///
/// | Strategy | Aliases | Parameters |
/// |---|---|---|
/// | RRF | `rrf` | `k` (default 60) |
/// | Relative Score | `rsf`, `relative_score` | `dense_w`, `sparse_w` (default 0.5 / 0.5) |
/// | Average | `average`, `avg` | — |
/// | Maximum | `maximum`, `max` | — |
/// | Weighted | `weighted` | `avg_w`, `max_w`, `hit_w` (default 0.5 / 0.3 / 0.2) |
///
/// Unknown strategies yield a 400 Bad Request response listing the
/// supported values. This propagates the full `velesdb_core::FusionStrategy`
/// enum to the REST surface (findings PROP-FUS-HYBRID / PROP-FUS-SPARSE).
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
                        code: None,
                    }),
                )
                    .into_response()
            })
        }
        "average" | "avg" => Ok(velesdb_core::FusionStrategy::Average),
        "maximum" | "max" => Ok(velesdb_core::FusionStrategy::Maximum),
        "weighted" => Ok(velesdb_core::FusionStrategy::Weighted {
            avg_weight: f.avg_w.unwrap_or(0.5),
            max_weight: f.max_w.unwrap_or(0.3),
            hit_weight: f.hit_w.unwrap_or(0.2),
        }),
        other => Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!(
                    "Invalid fusion strategy: '{other}'. Valid values: \
                     'rrf', 'rsf' (alias: 'relative_score'), \
                     'average' (alias: 'avg'), \
                     'maximum' (alias: 'max'), 'weighted'"
                ),
                code: None,
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

    // Quality-based mode (supports AutoTune which computes ef dynamically).
    // Supersedes mode_to_ef_search — all named modes map to SearchQuality.
    let quality_mode = req.mode.as_ref().and_then(|m| mode_to_search_quality(m));

    // Known limitation (#457): when filter is present, mode/ef_search are ignored
    // because search_with_filter does not accept a quality parameter yet.
    let result = if let Some(ref filter_json) = req.filter {
        let filter = parse_filter_or_400(filter_json, &state.onboarding_metrics)?;
        collection.search_with_filter(&req.vector, req.top_k, &filter)
    } else if let Some(ef) = req.ef_search {
        // Explicit ef_search takes precedence over quality mode
        collection.search_with_ef(&req.vector, req.top_k, ef)
    } else if let Some(quality) = quality_mode {
        collection.search_with_quality(&req.vector, req.top_k, quality)
    } else {
        collection.search(&req.vector, req.top_k)
    };
    Ok(result)
}

/// Search mode classification used by [`execute_search_request`] to
/// dispatch to the appropriate search backend.
///
/// The variants enumerate the three legitimate combinations of
/// dense and sparse query payloads on a `SearchRequest`:
/// - `Hybrid` : both a dense vector and a sparse vector are present.
/// - `DenseOnly` : only a dense vector is present.
/// - `SparseOnly` : only a sparse vector is present.
///
/// The "neither" case is rejected upfront with a 400 Bad Request,
/// so the enum does not carry a fallback variant.
///
/// # Exhaustiveness guarantee
///
/// Both [`SearchMode::classify`] and the `match` dispatch in
/// [`execute_search_request`] are exhaustive over this enum. Adding
/// a new search mode (e.g. a future `HybridLexical` combining
/// dense + BM25 + sparse) will trigger a compile error at both
/// sites instead of silently falling through to the legacy
/// dense-only code path — this is the audit A P1 finding S2-NEW-08
/// that motivated the refactor from the previous if-let chain.
enum SearchMode<'a> {
    /// Both dense and sparse vectors are present — route to the
    /// hybrid fusion backend.
    Hybrid {
        sparse: &'a velesdb_core::index::sparse::SparseVector,
    },
    /// Only a dense vector is present.
    DenseOnly,
    /// Only a sparse vector is present.
    SparseOnly {
        sparse: &'a velesdb_core::index::sparse::SparseVector,
    },
}

impl<'a> SearchMode<'a> {
    /// Classify a request by its dense/sparse payload presence.
    ///
    /// Returns a 400 Bad Request when neither is provided — the
    /// only invalid state. All three legitimate combinations map
    /// to a corresponding [`SearchMode`] variant.
    #[allow(clippy::result_large_err)]
    fn classify(
        has_dense: bool,
        sparse: Option<&'a velesdb_core::index::sparse::SparseVector>,
    ) -> Result<Self, axum::response::Response> {
        match (has_dense, sparse) {
            (true, Some(s)) => Ok(Self::Hybrid { sparse: s }),
            (true, None) => Ok(Self::DenseOnly),
            (false, Some(s)) => Ok(Self::SparseOnly { sparse: s }),
            (false, None) => Err((
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "Either 'vector' or 'sparse_vector' must be provided".to_string(),
                    code: None,
                }),
            )
                .into_response()),
        }
    }
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

    let index_name = req
        .sparse_index
        .as_deref()
        .unwrap_or(DEFAULT_SPARSE_INDEX_NAME);

    match SearchMode::classify(has_dense, sparse_vec.as_ref())? {
        SearchMode::Hybrid { sparse } => {
            execute_hybrid_sparse(state, name, collection, req, sparse, index_name)
        }
        SearchMode::DenseOnly => execute_dense_search(state, name, collection, req),
        SearchMode::SparseOnly { sparse } => {
            Ok(collection.sparse_search(sparse, req.top_k, index_name))
        }
    }
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
    let elapsed = start.elapsed();
    let duration_us = elapsed.as_micros();
    #[allow(clippy::cast_possible_truncation)]
    // Reason: value is clamped to u64::MAX above, so the truncation is lossless.
    state
        .db
        .notify_query(name, duration_us.min(u128::from(u64::MAX)) as u64);
    // Record into Prometheus histogram (seconds).
    state
        .query_duration_histogram
        .observe(elapsed.as_secs_f64());
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
        Err(e) => {
            state.operational_metrics.inc_errors();
            (error_status, Json(actionable_search_error(&e))).into_response()
        }
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

/// Builds the 408 Request Timeout response returned when a search
/// exceeds the per-request `timeout_ms` budget. Includes the collection
/// name and the budget in milliseconds so that clients can log
/// actionable diagnostics.
pub(crate) fn timeout_response(collection_name: &str, timeout_ms: u64) -> axum::response::Response {
    (
        StatusCode::REQUEST_TIMEOUT,
        Json(ErrorResponse {
            error: format!(
                "Search on collection '{collection_name}' exceeded the \
                 requested timeout of {timeout_ms}ms. The server returned \
                 early; the in-flight query may continue in the background \
                 until completion.",
            ),
            code: Some("VELES-QUERY-TIMEOUT".to_string()),
        }),
    )
        .into_response()
}

// Async worker wrappers (TimeoutElapsed, run_search_with_optional_timeout,
// run_blocking_search) extracted to workers.rs — Extract Module (Fowler).

#[cfg(test)]
mod parse_fusion_strategy_tests {
    use super::parse_fusion_strategy;
    use crate::types::FusionRequest;

    fn strat(name: &str) -> FusionRequest {
        FusionRequest {
            strategy: name.to_string(),
            k: None,
            dense_w: None,
            sparse_w: None,
            avg_w: None,
            max_w: None,
            hit_w: None,
        }
    }

    #[test]
    fn test_default_is_rrf_k60() {
        let result = parse_fusion_strategy(None).expect("None must default to RRF");
        match result {
            velesdb_core::FusionStrategy::RRF { k } => assert_eq!(k, 60),
            other => panic!("expected RRF, got {other:?}"),
        }
    }

    #[test]
    fn test_rrf_with_custom_k() {
        let mut req = strat("rrf");
        req.k = Some(120);
        let result = parse_fusion_strategy(Some(&req)).expect("rrf must parse");
        match result {
            velesdb_core::FusionStrategy::RRF { k } => assert_eq!(k, 120),
            other => panic!("expected RRF, got {other:?}"),
        }
    }

    #[test]
    fn test_average_no_params() {
        for alias in ["average", "avg", "AVG"] {
            let req = strat(alias);
            let result = parse_fusion_strategy(Some(&req))
                .unwrap_or_else(|_| panic!("'{alias}' must parse"));
            assert!(matches!(result, velesdb_core::FusionStrategy::Average));
        }
    }

    #[test]
    fn test_maximum_no_params() {
        for alias in ["maximum", "max", "MAX"] {
            let req = strat(alias);
            let result = parse_fusion_strategy(Some(&req))
                .unwrap_or_else(|_| panic!("'{alias}' must parse"));
            assert!(matches!(result, velesdb_core::FusionStrategy::Maximum));
        }
    }

    #[test]
    fn test_weighted_with_defaults() {
        let req = strat("weighted");
        let result = parse_fusion_strategy(Some(&req)).expect("weighted must parse");
        match result {
            velesdb_core::FusionStrategy::Weighted {
                avg_weight,
                max_weight,
                hit_weight,
            } => {
                assert!((avg_weight - 0.5).abs() < f32::EPSILON);
                assert!((max_weight - 0.3).abs() < f32::EPSILON);
                assert!((hit_weight - 0.2).abs() < f32::EPSILON);
            }
            other => panic!("expected Weighted, got {other:?}"),
        }
    }

    #[test]
    fn test_weighted_with_explicit_weights() {
        let mut req = strat("weighted");
        req.avg_w = Some(0.7);
        req.max_w = Some(0.2);
        req.hit_w = Some(0.1);
        let result = parse_fusion_strategy(Some(&req)).expect("weighted must parse");
        match result {
            velesdb_core::FusionStrategy::Weighted {
                avg_weight,
                max_weight,
                hit_weight,
            } => {
                assert!((avg_weight - 0.7).abs() < f32::EPSILON);
                assert!((max_weight - 0.2).abs() < f32::EPSILON);
                assert!((hit_weight - 0.1).abs() < f32::EPSILON);
            }
            other => panic!("expected Weighted, got {other:?}"),
        }
    }

    #[test]
    fn test_rsf_with_dense_weight_only() {
        let mut req = strat("rsf");
        req.dense_w = Some(0.7);
        let result = parse_fusion_strategy(Some(&req)).expect("rsf must parse");
        match result {
            velesdb_core::FusionStrategy::RelativeScore {
                dense_weight,
                sparse_weight,
            } => {
                assert!((dense_weight - 0.7).abs() < f32::EPSILON);
                assert!((sparse_weight - 0.3).abs() < f32::EPSILON);
            }
            other => panic!("expected RelativeScore, got {other:?}"),
        }
    }

    #[test]
    fn test_relative_score_alias() {
        let req = strat("relative_score");
        let result = parse_fusion_strategy(Some(&req)).expect("relative_score must parse");
        assert!(matches!(
            result,
            velesdb_core::FusionStrategy::RelativeScore { .. }
        ));
    }

    #[test]
    fn test_unknown_strategy_returns_error() {
        let req = strat("nonexistent");
        let result = parse_fusion_strategy(Some(&req));
        assert!(
            result.is_err(),
            "unknown strategy must return Err (400 response)"
        );
    }

    #[test]
    fn test_rrf_alias_case_insensitive() {
        for alias in ["rrf", "RRF", "Rrf"] {
            let req = strat(alias);
            let result = parse_fusion_strategy(Some(&req))
                .unwrap_or_else(|_| panic!("'{alias}' must parse"));
            assert!(matches!(result, velesdb_core::FusionStrategy::RRF { .. }));
        }
    }
}
