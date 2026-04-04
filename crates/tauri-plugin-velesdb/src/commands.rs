//! Tauri commands for `VelesDB` operations exposed via IPC.
#![allow(clippy::missing_errors_doc, deprecated)]

use crate::error::{CommandError, Error};
use crate::events::{emit_collection_created, emit_collection_deleted, emit_collection_updated};
#[cfg(feature = "persistence")]
use crate::helpers::require_vector_collection;
use crate::helpers::{
    map_core_results, metric_to_string, parse_filter, parse_fusion_strategy, parse_metric,
    parse_storage_mode, require_collection, storage_mode_to_string, timed_search_response,
};
use crate::state::VelesDbState;
#[cfg(feature = "persistence")]
use crate::types::StreamInsertRequest;
pub use crate::types::{
    default_fusion, default_metric, default_storage_mode, default_top_k, default_vector_weight,
};
use crate::types::{
    BatchSearchRequest, CollectionInfo, CreateCollectionRequest, CreateMetadataCollectionRequest,
    DeletePointsRequest, GetPointsRequest, HybridSearchRequest, MultiQuerySearchRequest,
    PointOutput, SearchRequest, SearchResponse, TextSearchRequest, TrainPqRequest,
    UpsertMetadataRequest, UpsertRequest,
};
use tauri::{command, AppHandle, Runtime, State};

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
            db.create_vector_collection_with_options(
                &request.name,
                request.dimension,
                metric,
                storage_mode,
            )?;
            Ok(CollectionInfo {
                name: request.name.clone(),
                dimension: request.dimension,
                metric: metric_to_string(metric).to_string(),
                count: 0,
                storage_mode: storage_mode_to_string(storage_mode).to_string(),
            })
        })
        .map_err(CommandError::from)?;

    // Emit event
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
                if let Some(coll) = db.get_collection(&name) {
                    let config = coll.config();
                    collections.push(CollectionInfo {
                        name,
                        dimension: config.dimension,
                        metric: metric_to_string(config.metric).to_string(),
                        count: coll.len(),
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

/// Searches for similar vectors.
#[command]
pub async fn search<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: SearchRequest,
) -> std::result::Result<SearchResponse, CommandError> {
    let start = std::time::Instant::now();
    let parsed_filter = parse_filter(&request.filter).map_err(CommandError::from)?;

    let results = state
        .with_db(|db| {
            let coll = require_collection(&db, &request.collection)?;

            let search_results = if let Some(ref f) = parsed_filter {
                coll.search_with_filter(&request.vector, request.top_k, f)?
            } else {
                coll.search(&request.vector, request.top_k)?
            };
            Ok(map_core_results(search_results))
        })
        .map_err(CommandError::from)?;

    Ok(timed_search_response(results, start))
}

/// Batch search for multiple query vectors in parallel.
#[command]
pub async fn batch_search<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: BatchSearchRequest,
) -> std::result::Result<Vec<SearchResponse>, CommandError> {
    let start = std::time::Instant::now();

    let batch_results = state
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
                .map(|s| {
                    s.filter
                        .as_ref()
                        .and_then(|f_json| serde_json::from_value(f_json.clone()).ok())
                })
                .collect();

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
        .map_err(CommandError::from)?;

    let timing_ms = start.elapsed().as_secs_f64() * 1000.0;
    Ok(batch_results
        .into_iter()
        .map(|results| SearchResponse { results, timing_ms })
        .collect())
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
                    None,
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

/// Multi-query fusion search combining results from multiple query vectors.
#[command]
pub async fn multi_query_search<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: MultiQuerySearchRequest,
) -> std::result::Result<SearchResponse, CommandError> {
    let start = std::time::Instant::now();
    let fusion_strategy = parse_fusion_strategy(&request.fusion, request.fusion_params.as_ref());
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
pub use crate::commands_memory::{semantic_query, semantic_store};

// NOTE: Secondary Index Commands moved to commands_index.rs
pub use crate::commands_index::{create_index, drop_index, list_indexes};

// NOTE: Knowledge Graph Commands moved to commands_graph.rs (EPIC-061/US-008 refactoring)
// Re-export graph commands for backwards compatibility
pub use crate::commands_graph::{add_edge, get_edges, get_node_degree, traverse_graph};

#[cfg(test)]
#[path = "commands_tests.rs"]
mod tests;
