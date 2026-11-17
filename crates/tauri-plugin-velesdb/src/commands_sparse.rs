//! Sparse vector Tauri commands extracted from `commands.rs`.
//!
//! Contains sparse search, hybrid sparse search, and sparse upsert commands.
#![allow(clippy::missing_errors_doc)]

use crate::error::CommandError;
use crate::helpers::{
    map_core_results, parse_sparse_vector, require_collection, require_vector_collection,
    timed_search_response,
};
use crate::state::VelesDbState;
use crate::types::{
    HybridSparseSearchRequest, SearchResponse, SparseSearchRequest, SparseUpsertRequest,
};
use tauri::{command, AppHandle, Runtime, State};

/// Searches using a sparse (keyword) vector via inverted index.
#[command]
pub async fn sparse_search<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: SparseSearchRequest,
) -> std::result::Result<SearchResponse, CommandError> {
    let start = std::time::Instant::now();

    let results = state
        .run_db(move |db| {
            let coll = require_vector_collection(&db, &request.collection)?;

            let core_sv = parse_sparse_vector(&request.sparse_vector)?;
            let idx_name = request.index_name.unwrap_or_default();

            crate::helpers::run_gated_search(
                &db,
                &request.collection,
                velesdb_core::observer::QueryOperationKind::VectorSearch,
                |scope| {
                    // A sparse-only search has no filter leaf to compose a
                    // scope into; narrowing its top-k afterwards returned
                    // fewer than `top_k` hits while matching points existed.
                    // Refused, as REST refuses it.
                    crate::helpers::deny_if_scoped(scope.as_ref(), "sparse-only search")?;
                    let search_results = coll.sparse_search(&core_sv, request.top_k, &idx_name)?;
                    Ok(map_core_results(search_results))
                },
            )
        })
        .await
        .map_err(CommandError::from)?;

    Ok(timed_search_response(results, start))
}

/// Performs hybrid dense+sparse search with RRF fusion.
#[command]
pub async fn hybrid_sparse_search<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: HybridSparseSearchRequest,
) -> std::result::Result<SearchResponse, CommandError> {
    let start = std::time::Instant::now();

    let results = state
        .run_db(move |db| {
            let coll = require_vector_collection(&db, &request.collection)?;

            let core_sv = parse_sparse_vector(&request.sparse_vector)?;
            let strategy = velesdb_core::fusion::FusionStrategy::rrf_default();

            crate::helpers::run_gated_search(
                &db,
                &request.collection,
                velesdb_core::observer::QueryOperationKind::VectorSearch,
                |scope| {
                    // The scope is core's filter: it narrows both branches
                    // before fusion, so the top-k is drawn from in-scope
                    // points only (narrowing the fused top-k afterwards
                    // under-filled it).
                    let search_results = coll.hybrid_sparse_search_filtered(
                        &request.vector,
                        &core_sv,
                        request.top_k,
                        "",
                        &strategy,
                        scope.as_ref(),
                    )?;
                    Ok(map_core_results(search_results))
                },
            )
        })
        .await
        .map_err(CommandError::from)?;

    Ok(timed_search_response(results, start))
}

/// Upserts points with optional sparse vectors.
#[command]
pub async fn sparse_upsert<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: SparseUpsertRequest,
) -> std::result::Result<usize, CommandError> {
    state
        .run_db(move |db| {
            let coll = require_collection(&db, &request.collection)?;

            let mut points = Vec::with_capacity(request.points.len());
            for p in request.points {
                let sparse_map = if let Some(ref sv) = p.sparse_vector {
                    let core_sv = parse_sparse_vector(sv)?;
                    let mut map = std::collections::BTreeMap::new();
                    map.insert(String::new(), core_sv);
                    Some(map)
                } else {
                    None
                };
                points.push(velesdb_core::Point::with_sparse(
                    p.id, p.vector, p.payload, sparse_map,
                ));
            }

            let count = points.len();
            coll.upsert(points)?;
            Ok(count)
        })
        .await
        .map_err(CommandError::from)
}
