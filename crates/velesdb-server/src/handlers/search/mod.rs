//! Search handlers for vector similarity, text, and hybrid search.

mod pipeline;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use std::sync::Arc;

use crate::types::{
    BatchSearchRequest, BatchSearchResponse, ErrorResponse, HybridSearchRequest,
    MultiQuerySearchRequest, SearchIdsResponse, SearchRequest, SearchResponse,
    SearchResultResponse, TextSearchRequest,
};
use crate::AppState;

use pipeline::{
    actionable_search_error, execute_search_request, finish_search, finish_search_ids,
    validate_query_dimension,
};

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
    Json(mut req): Json<SearchRequest>,
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

    let search_result = match execute_search_request(&state, &name, &collection, &mut req) {
        Ok(r) => r,
        Err(resp) => return resp,
    };

    finish_search(&state, &name, start, search_result)
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

    let max_top_k = req.searches.iter().map(|s| s.top_k).max().unwrap_or(10);

    let all_results = match collection.search_batch_with_filters(&queries, max_top_k, &filters) {
        Ok(batch_results) => {
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

    match results {
        Ok(results) => {
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
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(actionable_search_error(&e)),
        )
            .into_response(),
    }
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
        (status = 404, description = "Collection not found", body = ErrorResponse),
        (status = 400, description = "Invalid request", body = ErrorResponse)
    )
)]
#[allow(clippy::unused_async)]
pub async fn search_ids(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(mut req): Json<SearchRequest>,
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

    let search_result = match execute_search_request(&state, &name, &collection, &mut req) {
        Ok(r) => r,
        Err(resp) => return resp,
    };

    finish_search_ids(&state, &name, start, search_result)
}
