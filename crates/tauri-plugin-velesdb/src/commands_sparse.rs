//! Sparse vector Tauri commands extracted from `commands.rs`.
//!
//! Contains sparse search, hybrid sparse search, and sparse upsert commands.
#![allow(clippy::missing_errors_doc)]

use crate::error::CommandError;
use crate::events::emit_collection_updated;
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
        .with_db(|db| {
            let coll = require_vector_collection(&db, &request.collection)?;

            let core_sv = parse_sparse_vector(&request.sparse_vector)?;
            let idx_name = request.index_name.unwrap_or_default();

            let search_results = coll.sparse_search(&core_sv, request.top_k, &idx_name)?;
            Ok(map_core_results(search_results))
        })
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
        .with_db(|db| {
            let coll = require_vector_collection(&db, &request.collection)?;

            let core_sv = parse_sparse_vector(&request.sparse_vector)?;
            let strategy = velesdb_core::fusion::FusionStrategy::RRF { k: 60 };

            let search_results =
                coll.hybrid_sparse_search(&request.vector, &core_sv, request.top_k, "", &strategy)?;
            Ok(map_core_results(search_results))
        })
        .map_err(CommandError::from)?;

    Ok(timed_search_response(results, start))
}

/// Upserts points with optional sparse vectors.
#[command]
pub async fn sparse_upsert<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: SparseUpsertRequest,
) -> std::result::Result<usize, CommandError> {
    let collection_name = request.collection.clone();
    let count = state
        .with_db(|db| {
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
        .map_err(CommandError::from)?;

    emit_collection_updated(&app, &collection_name, "sparse_upsert", count);
    Ok(count)
}
