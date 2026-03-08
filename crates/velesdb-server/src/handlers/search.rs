//! Search handlers for vector similarity, text, and hybrid search.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use std::sync::Arc;
use velesdb_core::index::sparse::DEFAULT_SPARSE_INDEX_NAME;

use crate::types::{
    mode_to_ef_search, BatchSearchRequest, BatchSearchResponse, ErrorResponse, HybridSearchRequest,
    MultiQuerySearchRequest, SearchRequest, SearchResponse, SearchResultResponse,
    TextSearchRequest,
};
use crate::AppState;

fn dimension_mismatch_error(
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

fn validate_query_dimension(
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

fn actionable_search_error(error: &dyn std::fmt::Display) -> ErrorResponse {
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
        (status = 404, description = "Collection not found", body = ErrorResponse),
        (status = 400, description = "Invalid request", body = ErrorResponse)
    )
)]
#[allow(clippy::unused_async)]
pub async fn search(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(req): Json<SearchRequest>,
) -> impl IntoResponse {
    let start = std::time::Instant::now();
    state.onboarding_metrics.record_search_request();

    let collection = match state.db.get_vector_collection(&name) {
        Some(c) => c,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: format!("Collection '{}' not found", name),
                }),
            )
                .into_response()
        }
    };

    // Determine search mode from request fields.
    let has_dense = !req.vector.is_empty();
    // Resolve sparse query vector:
    //   1. `sparse_vector` (single, unnamed) takes priority.
    //   2. `sparse_vectors` with exactly one entry is accepted without `sparse_index`.
    //   3. `sparse_vectors` with >1 entry requires `sparse_index` to be specified;
    //      otherwise the request is ambiguous and is rejected with HTTP 400.
    let sparse_input = if let Some(sv) = req.sparse_vector {
        Some(sv)
    } else if let Some(mut m) = req.sparse_vectors {
        if m.len() > 1 && req.sparse_index.is_none() {
            return (
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
                .into_response();
        }
        // Use `sparse_index` as the key if provided; otherwise take the only entry.
        if let Some(ref idx_name) = req.sparse_index {
            m.remove(idx_name.as_str())
        } else {
            m.pop_first().map(|(_, v)| v)
        }
    } else {
        None
    };
    let has_sparse = sparse_input.is_some();

    if !has_dense && !has_sparse {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Either 'vector' or 'sparse_vector' must be provided".to_string(),
            }),
        )
            .into_response();
    }

    // Convert sparse input if present.
    let sparse_vec = if let Some(sv_input) = sparse_input {
        match sv_input.into_sparse_vector() {
            Ok(sv) => Some(sv),
            Err(e) => {
                return (StatusCode::BAD_REQUEST, Json(ErrorResponse { error: e })).into_response();
            }
        }
    } else {
        None
    };

    let index_name = req
        .sparse_index
        .as_deref()
        .unwrap_or(DEFAULT_SPARSE_INDEX_NAME);

    // ---- HYBRID: both dense and sparse ----
    if has_dense && has_sparse {
        let expected_dimension = collection.config().dimension;
        if let Err(error) = validate_query_dimension(&state, &name, expected_dimension, &req.vector)
        {
            return (StatusCode::BAD_REQUEST, Json(error)).into_response();
        }

        let strategy = match req.fusion {
            Some(ref f) => match f.strategy.to_lowercase().as_str() {
                "rrf" => velesdb_core::FusionStrategy::RRF {
                    k: f.k.unwrap_or(60),
                },
                // "rsf" is the canonical REST alias for `FusionStrategy::RelativeScore`
                // (maps to `FusionStrategyType::Rsf` in the core enum).
                // "relative_score" is accepted as a more descriptive synonym.
                "rsf" | "relative_score" => {
                    let (dw, sw) = match (f.dense_w, f.sparse_w) {
                        (Some(d), Some(s)) => (d, s),
                        (Some(d), None) => (d, 1.0 - d),
                        (None, Some(s)) => (1.0 - s, s),
                        (None, None) => (0.5, 0.5),
                    };
                    match velesdb_core::FusionStrategy::relative_score(dw, sw) {
                        Ok(s) => s,
                        Err(e) => {
                            return (
                                StatusCode::BAD_REQUEST,
                                Json(ErrorResponse {
                                    error: format!("Invalid RSF fusion weights: {e}"),
                                }),
                            )
                                .into_response();
                        }
                    }
                }
                other => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: format!(
                                "Invalid fusion strategy: '{other}'. \
                                 Valid values: 'rrf', 'rsf' (alias: 'relative_score')"
                            ),
                        }),
                    )
                        .into_response();
                }
            },
            None => velesdb_core::FusionStrategy::rrf_default(),
        };

        let sparse_query = sparse_vec.expect("sparse_vec is Some when has_sparse is true");
        let search_result = collection.hybrid_sparse_search(
            &req.vector,
            &sparse_query,
            req.top_k,
            index_name,
            &strategy,
        );

        return finish_search(&state, &name, start, search_result);
    }

    // ---- SPARSE-ONLY ----
    if has_sparse {
        let sparse_query = sparse_vec.expect("sparse_vec is Some when has_sparse is true");
        let search_result = collection.sparse_search(&sparse_query, req.top_k, index_name);
        return finish_search(&state, &name, start, search_result);
    }

    // ---- DENSE-ONLY (existing path) ----
    let expected_dimension = collection.config().dimension;
    if let Err(error) = validate_query_dimension(&state, &name, expected_dimension, &req.vector) {
        return (StatusCode::BAD_REQUEST, Json(error)).into_response();
    }

    let effective_ef = req
        .ef_search
        .or_else(|| req.mode.as_ref().and_then(|m| mode_to_ef_search(m)));

    let search_result = if let Some(ref filter_json) = req.filter {
        let filter: velesdb_core::Filter = match serde_json::from_value(filter_json.clone()) {
            Ok(f) => f,
            Err(e) => {
                state.onboarding_metrics.record_filter_parse_error();
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: format!("Invalid filter: {}", e),
                    }),
                )
                    .into_response();
            }
        };
        collection.search_with_filter(&req.vector, req.top_k, &filter)
    } else if let Some(ef) = effective_ef {
        collection.search_with_ef(&req.vector, req.top_k, ef)
    } else {
        collection.search(&req.vector, req.top_k)
    };

    finish_search(&state, &name, start, search_result)
}

/// Shared result-handling for all search modes.
fn finish_search(
    state: &AppState,
    name: &str,
    start: std::time::Instant,
    search_result: velesdb_core::Result<Vec<velesdb_core::SearchResult>>,
) -> axum::response::Response {
    match search_result {
        Ok(results) => {
            if results.is_empty() {
                state.onboarding_metrics.record_empty_search_results();
            }
            let duration_us = start.elapsed().as_micros();
            #[allow(clippy::cast_possible_truncation)]
            state
                .db
                .notify_query(name, duration_us.min(u128::from(u64::MAX)) as u64);

            let response = SearchResponse {
                results: results
                    .into_iter()
                    .map(|r| SearchResultResponse {
                        id: r.point.id,
                        score: r.score,
                        payload: r.point.payload,
                    })
                    .collect(),
            };
            Json(response).into_response()
        }
        Err(e) => (StatusCode::BAD_REQUEST, Json(actionable_search_error(&e))).into_response(),
    }
}

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
    Path(name): Path<String>,
    Json(req): Json<BatchSearchRequest>,
) -> impl IntoResponse {
    let start = std::time::Instant::now();
    state.onboarding_metrics.record_search_request();

    let collection = match state.db.get_vector_collection(&name) {
        Some(c) => c,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: format!("Collection '{}' not found", name),
                }),
            )
                .into_response()
        }
    };

    let expected_dimension = collection.config().dimension;
    for (idx, search) in req.searches.iter().enumerate() {
        if let Err(error) =
            validate_query_dimension(&state, &name, expected_dimension, &search.vector)
        {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!("Invalid query at index {idx}: {}", error.error),
                }),
            )
                .into_response();
        }
    }

    let queries: Vec<&[f32]> = req.searches.iter().map(|s| s.vector.as_slice()).collect();

    let mut filters: Vec<Option<velesdb_core::Filter>> = Vec::with_capacity(req.searches.len());
    for (idx, search) in req.searches.iter().enumerate() {
        if let Some(filter_json) = &search.filter {
            match serde_json::from_value(filter_json.clone()) {
                Ok(filter) => filters.push(Some(filter)),
                Err(e) => {
                    state.onboarding_metrics.record_filter_parse_error();
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: format!(
                                "Invalid filter at index {idx}: {e}. Hint: validate filter syntax and start with a broader query before reintroducing strict filters."
                            ),
                        }),
                    )
                        .into_response();
                }
            }
        } else {
            filters.push(None);
        }
    }

    let top_k = req.searches.first().map_or(10, |s| s.top_k);

    let all_results = match collection.search_batch_with_filters(&queries, top_k, &filters) {
        Ok(batch_results) => {
            let empty_count = batch_results
                .iter()
                .filter(|results| results.is_empty())
                .count();
            for _ in 0..empty_count {
                state.onboarding_metrics.record_empty_search_results();
            }
            batch_results
                .into_iter()
                .map(|results| SearchResponse {
                    results: results
                        .into_iter()
                        .map(|r| SearchResultResponse {
                            id: r.point.id,
                            score: r.score,
                            payload: r.point.payload,
                        })
                        .collect(),
                })
                .collect()
        }
        Err(e) => {
            return (StatusCode::BAD_REQUEST, Json(actionable_search_error(&e))).into_response()
        }
    };

    let timing_ms = start.elapsed().as_secs_f64() * 1000.0;
    let duration_us = start.elapsed().as_micros();
    #[allow(clippy::cast_possible_truncation)]
    state
        .db
        .notify_query(&name, duration_us.min(u128::from(u64::MAX)) as u64);

    Json(BatchSearchResponse {
        results: all_results,
        timing_ms,
    })
    .into_response()
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
#[allow(clippy::unused_async)]
pub async fn multi_query_search(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(req): Json<MultiQuerySearchRequest>,
) -> impl IntoResponse {
    use velesdb_core::FusionStrategy;
    state.onboarding_metrics.record_search_request();

    let collection = match state.db.get_vector_collection(&name) {
        Some(c) => c,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: format!("Collection '{}' not found", name),
                }),
            )
                .into_response()
        }
    };

    let strategy = match req.strategy.to_lowercase().as_str() {
        "average" | "avg" => FusionStrategy::Average,
        "maximum" | "max" => FusionStrategy::Maximum,
        "rrf" => FusionStrategy::RRF { k: req.rrf_k },
        "weighted" => FusionStrategy::Weighted {
            avg_weight: req.avg_weight,
            max_weight: req.max_weight,
            hit_weight: req.hit_weight,
        },
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!(
                        "Invalid strategy: {}. Valid: average, maximum, rrf, weighted",
                        req.strategy
                    ),
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
                }),
            )
                .into_response();
        }
    }

    let query_refs: Vec<&[f32]> = req.vectors.iter().map(Vec::as_slice).collect();

    let results = match collection.multi_query_search(&query_refs, req.top_k, strategy, None) {
        Ok(r) => r,
        Err(e) => {
            return (StatusCode::BAD_REQUEST, Json(actionable_search_error(&e))).into_response()
        }
    };

    if results.is_empty() {
        state.onboarding_metrics.record_empty_search_results();
    }

    let response = SearchResponse {
        results: results
            .into_iter()
            .map(|r| SearchResultResponse {
                id: r.point.id,
                score: r.score,
                payload: r.point.payload,
            })
            .collect(),
    };

    Json(response).into_response()
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
        (status = 404, description = "Collection not found", body = ErrorResponse)
    )
)]
#[allow(clippy::unused_async)]
pub async fn text_search(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(req): Json<TextSearchRequest>,
) -> impl IntoResponse {
    state.onboarding_metrics.record_search_request();

    let collection = match state.db.get_vector_collection(&name) {
        Some(c) => c,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: format!("Collection '{}' not found", name),
                }),
            )
                .into_response()
        }
    };

    let results = if let Some(ref filter_json) = req.filter {
        let filter: velesdb_core::Filter = match serde_json::from_value(filter_json.clone()) {
            Ok(f) => f,
            Err(e) => {
                state.onboarding_metrics.record_filter_parse_error();
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: format!("Invalid filter: {}", e),
                    }),
                )
                    .into_response();
            }
        };
        collection.text_search_with_filter(&req.query, req.top_k, &filter)
    } else {
        collection.text_search(&req.query, req.top_k)
    };

    let response = SearchResponse {
        results: results
            .into_iter()
            .map(|r| SearchResultResponse {
                id: r.point.id,
                score: r.score,
                payload: r.point.payload,
            })
            .collect(),
    };

    Json(response).into_response()
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
        (status = 404, description = "Collection not found", body = ErrorResponse),
        (status = 400, description = "Invalid request", body = ErrorResponse)
    )
)]
#[allow(clippy::unused_async)]
pub async fn hybrid_search(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(req): Json<HybridSearchRequest>,
) -> impl IntoResponse {
    state.onboarding_metrics.record_search_request();

    let collection = match state.db.get_vector_collection(&name) {
        Some(c) => c,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: format!("Collection '{}' not found", name),
                }),
            )
                .into_response()
        }
    };

    let expected_dimension = collection.config().dimension;
    if let Err(error) = validate_query_dimension(&state, &name, expected_dimension, &req.vector) {
        return (StatusCode::BAD_REQUEST, Json(error)).into_response();
    }

    let search_result = if let Some(ref filter_json) = req.filter {
        let filter: velesdb_core::Filter = match serde_json::from_value(filter_json.clone()) {
            Ok(f) => f,
            Err(e) => {
                state.onboarding_metrics.record_filter_parse_error();
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: format!("Invalid filter: {}", e),
                    }),
                )
                    .into_response();
            }
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

    match search_result {
        Ok(results) => {
            if results.is_empty() {
                state.onboarding_metrics.record_empty_search_results();
            }
            let response = SearchResponse {
                results: results
                    .into_iter()
                    .map(|r| SearchResultResponse {
                        id: r.point.id,
                        score: r.score,
                        payload: r.point.payload,
                    })
                    .collect(),
            };
            Json(response).into_response()
        }
        Err(e) => (StatusCode::BAD_REQUEST, Json(actionable_search_error(&e))).into_response(),
    }
}
