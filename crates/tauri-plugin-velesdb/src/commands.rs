//! Tauri commands for `VelesDB` operations exposed via IPC.
#![allow(clippy::missing_errors_doc, deprecated)]

use crate::error::{CommandError, Error};
use crate::events::{emit_collection_created, emit_collection_deleted, emit_collection_updated};
use crate::helpers::{
    map_core_results, metric_to_string, parse_filter, parse_fusion_strategy, parse_metric,
    parse_storage_mode, require_collection, storage_mode_to_string, timed_search_response,
};
#[cfg(feature = "persistence")]
use crate::helpers::{parse_search_quality, require_vector_collection};
use crate::state::VelesDbState;
#[cfg(feature = "persistence")]
use crate::types::StreamInsertRequest;
pub use crate::types::{
    default_fusion, default_metric, default_storage_mode, default_top_k, default_vector_weight,
};
use crate::types::{
    BatchSearchRequest, CollectionInfo, CreateCollectionRequest, CreateMetadataCollectionRequest,
    DeletePointsRequest, GetPointsRequest, HybridSearchRequest, MultiQuerySearchRequest,
    PointOutput, ScrollRequest, ScrollResponse, SearchRequest, SearchResponse, TextSearchRequest,
    TrainPqRequest, UpsertMetadataRequest, UpsertRequest,
};
use tauri::{command, AppHandle, Runtime, State};

/// Builds [`velesdb_core::HnswParams`] from optional request fields, falling
/// back to dimension-based auto-tuned defaults for any omitted parameter.
pub(crate) fn build_hnsw_params(
    request: &CreateCollectionRequest,
    storage_mode: velesdb_core::StorageMode,
) -> velesdb_core::HnswParams {
    let base = velesdb_core::HnswParams::auto(request.dimension);
    velesdb_core::HnswParams {
        max_connections: request.hnsw_m.unwrap_or(base.max_connections),
        ef_construction: request.hnsw_ef_construction.unwrap_or(base.ef_construction),
        max_elements: request.hnsw_max_elements.unwrap_or(base.max_elements),
        storage_mode,
        alpha: request.hnsw_alpha.unwrap_or(base.alpha),
    }
}

/// Returns `true` when the request carries at least one advanced HNSW or PQ
/// parameter, indicating that [`build_hnsw_params`] should be used.
pub(crate) const fn has_advanced_params(request: &CreateCollectionRequest) -> bool {
    request.hnsw_m.is_some()
        || request.hnsw_ef_construction.is_some()
        || request.hnsw_alpha.is_some()
        || request.hnsw_max_elements.is_some()
        || request.pq_rescore_oversampling.is_some()
}

/// Creates a new collection.
#[command]
pub async fn create_collection<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: CreateCollectionRequest,
) -> std::result::Result<CollectionInfo, CommandError> {
    let metric = parse_metric(&request.metric).map_err(CommandError::from)?;
    let storage_mode = parse_storage_mode(&request.storage_mode).map_err(CommandError::from)?;

    let result = state
        .with_db(|db| {
            if has_advanced_params(&request) {
                let hnsw_params = build_hnsw_params(&request, storage_mode);
                db.create_vector_collection_with_params(
                    &request.name,
                    request.dimension,
                    metric,
                    storage_mode,
                    hnsw_params,
                    request.pq_rescore_oversampling,
                )?;
            } else {
                db.create_vector_collection_with_options(
                    &request.name,
                    request.dimension,
                    metric,
                    storage_mode,
                )?;
            }
            Ok(CollectionInfo {
                name: request.name.clone(),
                dimension: request.dimension,
                metric: metric_to_string(metric).to_string(),
                count: 0,
                storage_mode: storage_mode_to_string(storage_mode).to_string(),
            })
        })
        .map_err(CommandError::from)?;

    emit_collection_created(&app, &request.name);
    Ok(result)
}

/// Creates a metadata-only collection (no vectors, just payloads).
#[command]
pub async fn create_metadata_collection<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: CreateMetadataCollectionRequest,
) -> std::result::Result<CollectionInfo, CommandError> {
    let result = state
        .with_db(|db| {
            db.create_metadata_collection(&request.name)?;
            Ok(CollectionInfo {
                name: request.name.clone(),
                dimension: 0,
                metric: "none".to_string(),
                count: 0,
                storage_mode: "metadata_only".to_string(),
            })
        })
        .map_err(CommandError::from)?;

    emit_collection_created(&app, &request.name);
    Ok(result)
}

/// Deletes a collection.
#[command]
pub async fn delete_collection<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    name: String,
) -> std::result::Result<(), CommandError> {
    state
        .with_db(|db| {
            db.delete_collection(&name)?;
            Ok(())
        })
        .map_err(CommandError::from)?;

    emit_collection_deleted(&app, &name);
    Ok(())
}

/// Lists all collections.
#[command]
pub async fn list_collections<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
) -> std::result::Result<Vec<CollectionInfo>, CommandError> {
    state
        .with_db(|db| {
            let names = db.list_collections();
            let mut collections = Vec::new();
            for name in names {
                if let Some(coll) = db.get_any_collection(&name) {
                    let config = coll.config();
                    collections.push(CollectionInfo {
                        name,
                        dimension: config.dimension,
                        metric: metric_to_string(config.metric).to_string(),
                        count: config.point_count,
                        storage_mode: storage_mode_to_string(config.storage_mode).to_string(),
                    });
                }
            }
            Ok(collections)
        })
        .map_err(CommandError::from)
}

/// Gets info about a specific collection.
#[command]
pub async fn get_collection<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    name: String,
) -> std::result::Result<CollectionInfo, CommandError> {
    state
        .with_db(|db| {
            let coll = require_collection(&db, &name)?;
            let config = coll.config();
            Ok(CollectionInfo {
                name,
                dimension: config.dimension,
                metric: metric_to_string(config.metric).to_string(),
                count: coll.len(),
                storage_mode: storage_mode_to_string(config.storage_mode).to_string(),
            })
        })
        .map_err(CommandError::from)
}

/// Upserts points into a collection.
#[command]
pub async fn upsert<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: UpsertRequest,
) -> std::result::Result<usize, CommandError> {
    let collection_name = request.collection.clone();
    let count = state
        .with_db(|db| {
            let coll = require_collection(&db, &request.collection)?;

            let points: Vec<velesdb_core::Point> = request
                .points
                .into_iter()
                .map(|p| velesdb_core::Point::new(p.id, p.vector, p.payload))
                .collect();

            let count = points.len();
            coll.upsert(points)?;
            Ok(count)
        })
        .map_err(CommandError::from)?;

    emit_collection_updated(&app, &collection_name, "upsert", count);
    Ok(count)
}

/// Upserts metadata-only points into a collection.
#[command]
pub async fn upsert_metadata<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: UpsertMetadataRequest,
) -> std::result::Result<usize, CommandError> {
    let collection_name = request.collection.clone();
    let count = state
        .with_db(|db| {
            let coll = require_collection(&db, &request.collection)?;

            let points: Vec<velesdb_core::Point> = request
                .points
                .into_iter()
                .map(|p| velesdb_core::Point::new(p.id, vec![], Some(p.payload)))
                .collect();

            let count = points.len();
            coll.upsert_metadata(points)?;
            Ok(count)
        })
        .map_err(CommandError::from)?;

    emit_collection_updated(&app, &collection_name, "upsert_metadata", count);
    Ok(count)
}

/// Gets points by their IDs.
#[command]
pub async fn get_points<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: GetPointsRequest,
) -> std::result::Result<Vec<Option<PointOutput>>, CommandError> {
    state
        .with_db(|db| {
            let coll = require_collection(&db, &request.collection)?;

            let points = coll.get(&request.ids);
            Ok(points
                .into_iter()
                .map(|opt| {
                    opt.map(|p| PointOutput {
                        id: p.id,
                        vector: p.vector,
                        payload: p.payload,
                    })
                })
                .collect())
        })
        .map_err(CommandError::from)
}

/// Deletes points by their IDs.
#[command]
pub async fn delete_points<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: DeletePointsRequest,
) -> std::result::Result<(), CommandError> {
    state
        .with_db(|db| {
            let coll = require_collection(&db, &request.collection)?;

            coll.delete(&request.ids)?;
            Ok(())
        })
        .map_err(CommandError::from)
}

/// Dispatches a single search with optional quality mode.
///
/// When quality is `Some`, delegates to `search_with_quality`;
/// otherwise falls back to the default `search`.
#[cfg(feature = "persistence")]
fn dispatch_quality_search(
    coll: &velesdb_core::VectorCollection,
    query: &[f32],
    k: usize,
    quality: Option<&velesdb_core::SearchQuality>,
) -> crate::error::Result<Vec<velesdb_core::SearchResult>> {
    if let Some(q) = quality {
        coll.search_with_quality(query, k, *q)
            .map_err(crate::error::Error::Database)
    } else {
        coll.search(query, k).map_err(crate::error::Error::Database)
    }
}

/// Dispatches a single search (no quality support without `persistence`).
#[cfg(not(feature = "persistence"))]
fn dispatch_quality_search(
    coll: &velesdb_core::VectorCollection,
    query: &[f32],
    k: usize,
    _quality: Option<&()>,
) -> crate::error::Result<Vec<velesdb_core::SearchResult>> {
    coll.search(query, k).map_err(crate::error::Error::Database)
}

/// Searches for similar vectors.
///
/// Supports an optional `quality` mode (e.g. "fast", "balanced", "accurate",
/// "perfect", "auto", "custom:\<ef\>", "adaptive:\<min\>:\<max\>").
/// Known limitation (#457): when a filter is present, quality is ignored
/// because `search_with_filter` does not accept a quality parameter yet.
#[command]
pub async fn search<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: SearchRequest,
) -> std::result::Result<SearchResponse, CommandError> {
    let start = std::time::Instant::now();
    let parsed_filter = parse_filter(&request.filter).map_err(CommandError::from)?;
    #[cfg(feature = "persistence")]
    let parsed_quality = parse_search_quality(&request.quality).map_err(CommandError::from)?;
    #[cfg(not(feature = "persistence"))]
    let parsed_quality: Option<()> = None;

    let results = state
        .with_db(|db| {
            let coll = require_collection(&db, &request.collection)?;

            // Filter takes precedence (known limitation #457: quality ignored
            // when filter is present). Quality dispatch only when no filter.
            let search_results = if let Some(ref f) = parsed_filter {
                coll.search_with_filter(&request.vector, request.top_k, f)?
            } else {
                dispatch_quality_search(
                    &coll,
                    &request.vector,
                    request.top_k,
                    parsed_quality.as_ref(),
                )?
            };
            Ok(map_core_results(search_results))
        })
        .map_err(CommandError::from)?;

    Ok(timed_search_response(results, start))
}

/// Batch search for multiple query vectors.
///
/// When any individual search specifies a `quality` mode, each search is
/// dispatched individually (per-search quality). Otherwise the optimized
/// `search_batch_with_filters` batch API is used.
#[command]
pub async fn batch_search<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: BatchSearchRequest,
) -> std::result::Result<Vec<SearchResponse>, CommandError> {
    let start = std::time::Instant::now();
    let has_quality = request.searches.iter().any(|s| s.quality.is_some());

    let batch_results = if has_quality {
        batch_search_per_query(&state, &request)?
    } else {
        batch_search_bulk(&state, &request)?
    };

    let timing_ms = start.elapsed().as_secs_f64() * 1000.0;
    Ok(batch_results
        .into_iter()
        .map(|results| SearchResponse { results, timing_ms })
        .collect())
}

/// Per-query batch search with per-search quality dispatch.
fn batch_search_per_query(
    state: &VelesDbState,
    request: &BatchSearchRequest,
) -> std::result::Result<Vec<Vec<crate::types::SearchResult>>, CommandError> {
    state
        .with_db(|db| {
            let coll = require_collection(&db, &request.collection)?;
            let mut all_results = Vec::with_capacity(request.searches.len());
            for s in &request.searches {
                let filter = parse_filter(&s.filter)?;
                #[cfg(feature = "persistence")]
                let quality = parse_search_quality(&s.quality)
                    .map_err(|e| Error::InvalidConfig(e.to_string()))?;
                #[cfg(not(feature = "persistence"))]
                let quality: Option<()> = None;

                let search_results = if let Some(ref f) = filter {
                    coll.search_with_filter(&s.vector, s.top_k, f)?
                } else {
                    dispatch_quality_search(&coll, &s.vector, s.top_k, quality.as_ref())?
                };
                all_results.push(
                    search_results
                        .into_iter()
                        .map(crate::helpers::map_core_result)
                        .collect(),
                );
            }
            Ok(all_results)
        })
        .map_err(CommandError::from)
}

/// Optimized bulk batch search (no per-query quality).
fn batch_search_bulk(
    state: &VelesDbState,
    request: &BatchSearchRequest,
) -> std::result::Result<Vec<Vec<crate::types::SearchResult>>, CommandError> {
    state
        .with_db(|db| {
            let coll = require_collection(&db, &request.collection)?;

            let query_refs: Vec<&[f32]> = request
                .searches
                .iter()
                .map(|s| s.vector.as_slice())
                .collect();
            let filters: Vec<Option<velesdb_core::Filter>> = request
                .searches
                .iter()
                .map(|s| parse_filter(&s.filter))
                .collect::<crate::error::Result<_>>()?;

            // Use the maximum top_k across all searches so that every individual
            // query retrieves enough candidates before per-query truncation.
            let top_k = request.searches.iter().map(|s| s.top_k).max().unwrap_or(10);
            let results = coll.search_batch_with_filters(&query_refs, top_k, &filters)?;

            Ok(results
                .into_iter()
                .zip(request.searches.iter().map(|s| s.top_k))
                .map(|(search_results, k)| {
                    search_results
                        .into_iter()
                        .take(k)
                        .map(crate::helpers::map_core_result)
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>())
        })
        .map_err(CommandError::from)
}

/// Searches by text using BM25.
#[command]
pub async fn text_search<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: TextSearchRequest,
) -> std::result::Result<SearchResponse, CommandError> {
    let start = std::time::Instant::now();
    let parsed_filter = parse_filter(&request.filter).map_err(CommandError::from)?;

    let results = state
        .with_db(|db| {
            let coll = require_collection(&db, &request.collection)?;

            let search_results = if let Some(ref f) = parsed_filter {
                coll.text_search_with_filter(&request.query, request.top_k, f)?
            } else {
                coll.text_search(&request.query, request.top_k)?
            };
            Ok(map_core_results(search_results))
        })
        .map_err(CommandError::from)?;

    Ok(timed_search_response(results, start))
}

/// Hybrid search combining vector similarity and BM25.
#[command]
pub async fn hybrid_search<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: HybridSearchRequest,
) -> std::result::Result<SearchResponse, CommandError> {
    let start = std::time::Instant::now();
    let parsed_filter = parse_filter(&request.filter).map_err(CommandError::from)?;

    let results = state
        .with_db(|db| {
            let coll = require_collection(&db, &request.collection)?;

            let search_results = if let Some(ref f) = parsed_filter {
                coll.hybrid_search_with_filter(
                    &request.vector,
                    &request.query,
                    request.top_k,
                    Some(request.vector_weight),
                    f,
                )?
            } else {
                coll.hybrid_search(
                    &request.vector,
                    &request.query,
                    request.top_k,
                    Some(request.vector_weight),
                )?
            };
            Ok(map_core_results(search_results))
        })
        .map_err(CommandError::from)?;

    Ok(timed_search_response(results, start))
}

// NOTE: VelesQL query command moved to commands_query.rs (NLOC refactoring)
pub use crate::commands_query::query;

/// Checks if a collection is empty.
#[command]
pub async fn is_empty<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    name: String,
) -> std::result::Result<bool, CommandError> {
    state
        .with_db(|db| {
            let coll = require_collection(&db, &name)?;
            Ok(coll.is_empty())
        })
        .map_err(CommandError::from)
}

/// Flushes pending changes to disk for a collection.
#[command]
pub async fn flush<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    name: String,
) -> std::result::Result<(), CommandError> {
    state
        .with_db(|db| {
            let coll = require_collection(&db, &name)?;
            coll.flush()?;
            Ok(())
        })
        .map_err(CommandError::from)
}

/// Scrolls through collection points with cursor-based pagination.
#[command]
pub async fn scroll_collection<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: ScrollRequest,
) -> std::result::Result<ScrollResponse, CommandError> {
    let parsed_filter = parse_filter(&request.filter).map_err(CommandError::from)?;

    state
        .with_db(|db| {
            let coll = require_collection(&db, &request.collection)?;
            let batch =
                coll.scroll_batch(request.cursor, request.batch_size, parsed_filter.as_ref())?;
            Ok(ScrollResponse {
                points: batch
                    .points
                    .into_iter()
                    .map(|p| PointOutput {
                        id: p.id,
                        vector: p.vector,
                        payload: p.payload,
                    })
                    .collect(),
                next_cursor: batch.next_cursor,
            })
        })
        .map_err(CommandError::from)
}

/// Multi-query fusion search combining results from multiple query vectors.
#[command]
pub async fn multi_query_search<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: MultiQuerySearchRequest,
) -> std::result::Result<SearchResponse, CommandError> {
    let start = std::time::Instant::now();
    let fusion_strategy = parse_fusion_strategy(&request.fusion, request.fusion_params.as_ref())
        .map_err(CommandError::from)?;
    let parsed_filter = parse_filter(&request.filter).map_err(CommandError::from)?;

    let results = state
        .with_db(|db| {
            let coll = require_collection(&db, &request.collection)?;

            let vector_refs: Vec<&[f32]> = request.vectors.iter().map(Vec::as_slice).collect();

            let search_results = coll.multi_query_search(
                &vector_refs,
                request.top_k,
                fusion_strategy,
                parsed_filter.as_ref(),
            )?;

            Ok(map_core_results(search_results))
        })
        .map_err(CommandError::from)?;

    Ok(timed_search_response(results, start))
}

// NOTE: Sparse Vector Commands moved to commands_sparse.rs (NLOC refactoring)
pub use crate::commands_sparse::{hybrid_sparse_search, sparse_search, sparse_upsert};

// ============================================================================
// PQ Training Command
// ============================================================================

/// Trains a Product Quantizer on a collection via `VelesQL` TRAIN statement.
#[command]
pub async fn train_pq<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: TrainPqRequest,
) -> std::result::Result<String, CommandError> {
    state
        .with_db(|db| {
            use velesdb_core::velesql::{Query, TrainStatement, WithValue};

            let mut params = std::collections::HashMap::new();
            if let Some(m) = request.m {
                params.insert(
                    "m".to_string(),
                    WithValue::Integer(i64::try_from(m).unwrap_or(i64::MAX)),
                );
            }
            if let Some(k) = request.k {
                params.insert(
                    "k".to_string(),
                    WithValue::Integer(i64::try_from(k).unwrap_or(i64::MAX)),
                );
            }
            if let Some(true) = request.opq {
                params.insert("type".to_string(), WithValue::Identifier("opq".to_string()));
            }

            let query = Query::new_train(TrainStatement {
                collection: request.collection,
                params,
            });

            let empty_params = std::collections::HashMap::new();
            db.execute_query(&query, &empty_params)
                .map_err(|e| Error::InvalidConfig(format!("PQ training failed: {e}")))?;

            Ok("PQ training complete".to_string())
        })
        .map_err(CommandError::from)
}

// ============================================================================
// Streaming Insert Command
// ============================================================================

/// Stream-inserts points into a collection's delta buffer.
///
/// Uses the streaming ingestion pipeline for low-latency writes.
/// Requires the `persistence` feature and an active stream ingester.
#[cfg(feature = "persistence")]
#[command]
pub async fn stream_insert<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: StreamInsertRequest,
) -> std::result::Result<usize, CommandError> {
    let collection_name = request.collection.clone();
    let count = state
        .with_db(|db| {
            let coll = require_vector_collection(&db, &request.collection)?;

            let mut inserted = 0;
            for p in request.points {
                let point = velesdb_core::Point::new(p.id, p.vector, p.payload);
                coll.stream_insert(point).map_err(|e| {
                    Error::InvalidConfig(format!("Stream insert backpressure: {e}"))
                })?;
                inserted += 1;
            }
            Ok(inserted)
        })
        .map_err(CommandError::from)?;

    emit_collection_updated(&app, &collection_name, "stream_insert", count);
    Ok(count)
}

// NOTE: AgentMemory Commands moved to commands_memory.rs (NLOC refactoring)
pub use crate::commands_memory::{
    episodic_recent, episodic_record, procedural_learn, procedural_recall, semantic_query,
    semantic_store,
};

// NOTE: Secondary Index Commands moved to commands_index.rs
pub use crate::commands_index::{create_index, drop_index, list_indexes};

// NOTE: Knowledge Graph Commands moved to commands_graph.rs (EPIC-061/US-008 refactoring)
// Re-export graph commands for backwards compatibility
pub use crate::commands_graph::{
    add_edge, create_graph_collection, get_edges, get_node_degree, traverse_graph,
};

#[cfg(test)]
#[path = "commands_tests.rs"]
mod tests;
