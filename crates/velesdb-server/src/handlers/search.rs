//! Search handlers for vector similarity, text, and hybrid search.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use std::sync::Arc;

use crate::types::{
    mode_to_ef_search, BatchSearchRequest, BatchSearchResponse, ErrorResponse, HybridSearchRequest,
    MultiQuerySearchRequest, SearchRequest, SearchResponse, SearchResultResponse,
    TextSearchRequest,
};
use crate::AppState;

use super::helpers::{
    get_collection_or_404, internal_error, validate_query_non_empty, validate_top_k,
};

/// Search for similar vectors.
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
pub async fn search(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(req): Json<SearchRequest>,
) -> impl IntoResponse {
    let collection = match get_collection_or_404(&state, &name) {
        Ok(c) => c,
        Err(e) => return e.into_response(),
    };

    if let Err(e) = validate_top_k(req.top_k) {
        return e.into_response();
    }

    let effective_ef = req
        .ef_search
        .or_else(|| req.mode.as_ref().and_then(|m| mode_to_ef_search(m)));

    // Parse filter before spawn_blocking (serde is fast)
    let filter: Option<velesdb_core::Filter> = if let Some(ref filter_json) = req.filter {
        match serde_json::from_value(filter_json.clone()) {
            Ok(f) => Some(f),
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: format!("Invalid filter: {}", e),
                    }),
                )
                    .into_response()
            }
        }
    } else {
        None
    };

    let include_vectors = req.include_vectors;

    let result = tokio::task::spawn_blocking(move || {
        let search_result = if let Some(ref f) = filter {
            collection.search_with_filter(&req.vector, req.top_k, f)
        } else if let Some(ef) = effective_ef {
            collection.search_with_ef(&req.vector, req.top_k, ef)
        } else {
            collection.search(&req.vector, req.top_k)
        };

        search_result.map(|results| SearchResponse {
            results: results
                .into_iter()
                .map(|r| SearchResultResponse {
                    id: r.point.id,
                    score: r.score,
                    payload: r.point.payload,
                    vector: if include_vectors {
                        Some(r.point.vector)
                    } else {
                        None
                    },
                })
                .collect(),
        })
    })
    .await;

    match result {
        Ok(Ok(response)) => Json(response).into_response(),
        Ok(Err(e)) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
        Err(e) => internal_error("Search", &e).into_response(),
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
pub async fn batch_search(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(req): Json<BatchSearchRequest>,
) -> impl IntoResponse {
    let start = std::time::Instant::now();

    let collection = match get_collection_or_404(&state, &name) {
        Ok(c) => c,
        Err(e) => return e.into_response(),
    };

    // Validate top_k from first search in batch
    if let Some(first) = req.searches.first() {
        if let Err(e) = validate_top_k(first.top_k) {
            return e.into_response();
        }
    }

    // Parse filters before spawn_blocking
    let filters: Vec<Option<velesdb_core::Filter>> = req
        .searches
        .iter()
        .map(|s| {
            s.filter
                .as_ref()
                .and_then(|f_json| serde_json::from_value(f_json.clone()).ok())
        })
        .collect();

    let top_k = req.searches.first().map_or(10, |s| s.top_k);
    let include_vectors: Vec<bool> = req.searches.iter().map(|s| s.include_vectors).collect();

    let result = tokio::task::spawn_blocking(move || {
        let queries: Vec<&[f32]> = req.searches.iter().map(|s| s.vector.as_slice()).collect();
        collection
            .search_batch_with_filters(&queries, top_k, &filters)
            .map(|batch_results| {
                batch_results
                    .into_iter()
                    .zip(include_vectors)
                    .map(|(results, include_vectors)| SearchResponse {
                        results: results
                            .into_iter()
                            .map(|r| SearchResultResponse {
                                id: r.point.id,
                                score: r.score,
                                payload: r.point.payload,
                                vector: if include_vectors {
                                    Some(r.point.vector)
                                } else {
                                    None
                                },
                            })
                            .collect(),
                    })
                    .collect::<Vec<_>>()
            })
    })
    .await;

    let timing_ms = start.elapsed().as_secs_f64() * 1000.0;

    match result {
        Ok(Ok(all_results)) => Json(BatchSearchResponse {
            results: all_results,
            timing_ms,
        })
        .into_response(),
        Ok(Err(e)) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
        Err(e) => internal_error("Batch search", &e).into_response(),
    }
}

/// Multi-query search with fusion strategies.
pub async fn multi_query_search(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(req): Json<MultiQuerySearchRequest>,
) -> impl IntoResponse {
    use velesdb_core::FusionStrategy;

    let collection = match get_collection_or_404(&state, &name) {
        Ok(c) => c,
        Err(e) => return e.into_response(),
    };

    if let Err(e) = validate_top_k(req.top_k) {
        return e.into_response();
    }

    // Parse strategy before spawn_blocking (validation only)
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

    let top_k = req.top_k;
    let vectors = req.vectors;

    let result = tokio::task::spawn_blocking(move || {
        let query_refs: Vec<&[f32]> = vectors.iter().map(Vec::as_slice).collect();
        collection
            .multi_query_search(&query_refs, top_k, strategy, None)
            .map(|results| SearchResponse {
                results: results
                    .into_iter()
                    .map(|r| SearchResultResponse {
                        id: r.point.id,
                        score: r.score,
                        payload: r.point.payload,
                        vector: None,
                    })
                    .collect(),
            })
    })
    .await;

    match result {
        Ok(Ok(response)) => Json(response).into_response(),
        Ok(Err(e)) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
        Err(e) => internal_error("Multi-query search", &e).into_response(),
    }
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
pub async fn text_search(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(req): Json<TextSearchRequest>,
) -> impl IntoResponse {
    let collection = match get_collection_or_404(&state, &name) {
        Ok(c) => c,
        Err(e) => return e.into_response(),
    };

    if let Err(e) = validate_top_k(req.top_k) {
        return e.into_response();
    }
    if let Err(e) = validate_query_non_empty(&req.query) {
        return e.into_response();
    }

    // Parse filter before spawn_blocking
    let filter: Option<velesdb_core::Filter> = if let Some(ref filter_json) = req.filter {
        match serde_json::from_value(filter_json.clone()) {
            Ok(f) => Some(f),
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: format!("Invalid filter: {}", e),
                    }),
                )
                    .into_response()
            }
        }
    } else {
        None
    };

    let result = tokio::task::spawn_blocking(move || {
        let results = if let Some(ref f) = filter {
            collection.text_search_with_filter(&req.query, req.top_k, f)
        } else {
            collection.text_search(&req.query, req.top_k)
        };

        SearchResponse {
            results: results
                .into_iter()
                .map(|r| SearchResultResponse {
                    id: r.point.id,
                    score: r.score,
                    payload: r.point.payload,
                    vector: None,
                })
                .collect(),
        }
    })
    .await;

    match result {
        Ok(response) => Json(response).into_response(),
        Err(e) => internal_error("Text search", &e).into_response(),
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
pub async fn hybrid_search(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(req): Json<HybridSearchRequest>,
) -> impl IntoResponse {
    let collection = match get_collection_or_404(&state, &name) {
        Ok(c) => c,
        Err(e) => return e.into_response(),
    };

    if let Err(e) = validate_top_k(req.top_k) {
        return e.into_response();
    }
    if let Err(e) = validate_query_non_empty(&req.query) {
        return e.into_response();
    }

    // Parse filter before spawn_blocking
    let filter: Option<velesdb_core::Filter> = if let Some(ref filter_json) = req.filter {
        match serde_json::from_value(filter_json.clone()) {
            Ok(f) => Some(f),
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: format!("Invalid filter: {}", e),
                    }),
                )
                    .into_response()
            }
        }
    } else {
        None
    };

    let result = tokio::task::spawn_blocking(move || {
        let search_result = if let Some(ref f) = filter {
            collection.hybrid_search_with_filter(
                &req.vector,
                &req.query,
                req.top_k,
                Some(req.vector_weight),
                f,
            )
        } else {
            collection.hybrid_search(&req.vector, &req.query, req.top_k, Some(req.vector_weight))
        };

        search_result.map(|results| SearchResponse {
            results: results
                .into_iter()
                .map(|r| SearchResultResponse {
                    id: r.point.id,
                    score: r.score,
                    payload: r.point.payload,
                    vector: None,
                })
                .collect(),
        })
    })
    .await;

    match result {
        Ok(Ok(response)) => Json(response).into_response(),
        Ok(Err(e)) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
        Err(e) => internal_error("Hybrid search", &e).into_response(),
    }
}
