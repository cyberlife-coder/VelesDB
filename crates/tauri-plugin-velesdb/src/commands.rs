//! Tauri commands for `VelesDB` operations exposed via IPC.
#![allow(clippy::missing_errors_doc)]

use crate::error::{CommandError, Error};
use crate::events::{emit_collection_created, emit_collection_deleted, emit_collection_updated};
use crate::helpers::{
    metric_to_string, parse_fusion_strategy, parse_metric, parse_storage_mode,
    storage_mode_to_string,
};
use crate::state::VelesDbState;
pub use crate::types::{
    default_fusion, default_metric, default_storage_mode, default_top_k, default_vector_weight,
};
use crate::types::{
    BatchSearchRequest, CollectionInfo, CreateCollectionRequest, CreateMetadataCollectionRequest,
    DeletePointsRequest, GetPointsRequest, HybridResult, HybridSearchRequest,
    MultiQuerySearchRequest, PointOutput, QueryRequest, QueryResponse, SearchRequest,
    SearchResponse, SearchResult, TextSearchRequest, UpsertMetadataRequest, UpsertRequest,
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
            db.create_collection_with_options(
                &request.name,
                request.dimension,
                metric,
                storage_mode,
            )?;
            Ok(CollectionInfo {
                name: request.name.clone(),
                dimension: request.dimension,
                metric: metric_to_string(metric),
                count: 0,
                storage_mode: storage_mode_to_string(storage_mode),
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
            db.create_collection_typed(&request.name, &velesdb_core::CollectionType::MetadataOnly)?;
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
                        metric: metric_to_string(config.metric),
                        count: coll.len(),
                        storage_mode: storage_mode_to_string(config.storage_mode),
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
            let coll = db
                .get_collection(&name)
                .ok_or_else(|| Error::CollectionNotFound(name.clone()))?;
            let config = coll.config();
            Ok(CollectionInfo {
                name,
                dimension: config.dimension,
                metric: metric_to_string(config.metric),
                count: coll.len(),
                storage_mode: storage_mode_to_string(config.storage_mode),
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
            let coll = db
                .get_collection(&request.collection)
                .ok_or_else(|| Error::CollectionNotFound(request.collection.clone()))?;

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
            let coll = db
                .get_collection(&request.collection)
                .ok_or_else(|| Error::CollectionNotFound(request.collection.clone()))?;

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
            let coll = db
                .get_collection(&request.collection)
                .ok_or_else(|| Error::CollectionNotFound(request.collection.clone()))?;

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
            let coll = db
                .get_collection(&request.collection)
                .ok_or_else(|| Error::CollectionNotFound(request.collection.clone()))?;

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

    let filter = request.filter.clone();

    let results = state
        .with_db(|db| {
            let coll = db
                .get_collection(&request.collection)
                .ok_or_else(|| Error::CollectionNotFound(request.collection.clone()))?;

            let search_results = if let Some(ref filter_json) = filter {
                let filter: velesdb_core::Filter = serde_json::from_value(filter_json.clone())
                    .map_err(|e| Error::InvalidConfig(format!("Invalid filter: {e}")))?;
                coll.search_with_filter(&request.vector, request.top_k, &filter)?
            } else {
                coll.search(&request.vector, request.top_k)?
            };
            Ok(search_results
                .into_iter()
                .map(|r| SearchResult {
                    id: r.point.id,
                    score: r.score,
                    payload: r.point.payload,
                })
                .collect::<Vec<_>>())
        })
        .map_err(CommandError::from)?;

    Ok(SearchResponse {
        results,
        timing_ms: start.elapsed().as_secs_f64() * 1000.0,
    })
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
            let coll = db
                .get_collection(&request.collection)
                .ok_or_else(|| Error::CollectionNotFound(request.collection.clone()))?;

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

            // Use the top_k from the first search as default for the batch operation if needed,
            // though search_batch_with_filters will handle them correctly if we adapt it or use it as base.
            // For now, we'll use search_batch_with_filters from core.
            let top_k = request.searches.first().map_or(10, |s| s.top_k);
            let results = coll.search_batch_with_filters(&query_refs, top_k, &filters)?;

            Ok(results
                .into_iter()
                .map(|search_results| {
                    search_results
                        .into_iter()
                        .map(|r| SearchResult {
                            id: r.point.id,
                            score: r.score,
                            payload: r.point.payload,
                        })
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

    let filter = request.filter.clone();

    let results = state
        .with_db(|db| {
            let coll = db
                .get_collection(&request.collection)
                .ok_or_else(|| Error::CollectionNotFound(request.collection.clone()))?;

            let search_results = if let Some(ref filter_json) = filter {
                let filter: velesdb_core::Filter = serde_json::from_value(filter_json.clone())
                    .map_err(|e| Error::InvalidConfig(format!("Invalid filter: {e}")))?;
                coll.text_search_with_filter(&request.query, request.top_k, &filter)
            } else {
                coll.text_search(&request.query, request.top_k)
            };
            Ok(search_results
                .into_iter()
                .map(|r| SearchResult {
                    id: r.point.id,
                    score: r.score,
                    payload: r.point.payload,
                })
                .collect::<Vec<_>>())
        })
        .map_err(CommandError::from)?;

    Ok(SearchResponse {
        results,
        timing_ms: start.elapsed().as_secs_f64() * 1000.0,
    })
}

/// Hybrid search combining vector similarity and BM25.
#[command]
pub async fn hybrid_search<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: HybridSearchRequest,
) -> std::result::Result<SearchResponse, CommandError> {
    let start = std::time::Instant::now();

    let filter = request.filter.clone();

    let results = state
        .with_db(|db| {
            let coll = db
                .get_collection(&request.collection)
                .ok_or_else(|| Error::CollectionNotFound(request.collection.clone()))?;

            let search_results = if let Some(ref filter_json) = filter {
                let filter: velesdb_core::Filter = serde_json::from_value(filter_json.clone())
                    .map_err(|e| Error::InvalidConfig(format!("Invalid filter: {e}")))?;
                coll.hybrid_search_with_filter(
                    &request.vector,
                    &request.query,
                    request.top_k,
                    Some(request.vector_weight),
                    &filter,
                )?
            } else {
                coll.hybrid_search(
                    &request.vector,
                    &request.query,
                    request.top_k,
                    Some(request.vector_weight),
                )?
            };
            Ok(search_results
                .into_iter()
                .map(|r| SearchResult {
                    id: r.point.id,
                    score: r.score,
                    payload: r.point.payload,
                })
                .collect::<Vec<_>>())
        })
        .map_err(CommandError::from)?;

    Ok(SearchResponse {
        results,
        timing_ms: start.elapsed().as_secs_f64() * 1000.0,
    })
}

/// Executes a `VelesQL` query (EPIC-031 US-012).
///
/// Supports SELECT-style `VelesQL` queries with vector similarity search.
/// Returns results in `HybridResult` format for forward compatibility.
///
/// Note: Currently supports SELECT syntax only. MATCH/graph traversal
/// syntax is planned for a future release (see EPIC-010). For SELECT
/// queries, `graph_score` and `column_data` will be None.
#[command]
pub async fn query<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: QueryRequest,
) -> std::result::Result<QueryResponse, CommandError> {
    let start = std::time::Instant::now();

    // Parse the VelesQL query
    let parsed = velesdb_core::velesql::Parser::parse(&request.query)
        .map_err(|e| Error::InvalidConfig(format!("VelesQL parse error: {}", e.message)))?;

    let collection_name = &parsed.select.from;

    let results = state
        .with_db(|db| {
            let coll = db
                .get_collection(collection_name)
                .ok_or_else(|| Error::CollectionNotFound(collection_name.clone()))?;

            // Use unified execute_query from Collection
            let search_results = coll
                .execute_query(&parsed, &request.params)
                .map_err(|e| Error::InvalidConfig(format!("Query execution error: {e}")))?;

            // Return multi-model HybridResult format
            Ok(search_results
                .into_iter()
                .map(|r| HybridResult {
                    node_id: r.point.id,
                    vector_score: Some(r.score),
                    graph_score: None,
                    fused_score: r.score,
                    bindings: r.point.payload.clone(),
                    column_data: None,
                })
                .collect::<Vec<_>>())
        })
        .map_err(CommandError::from)?;

    Ok(QueryResponse {
        results,
        timing_ms: start.elapsed().as_secs_f64() * 1000.0,
    })
}

/// Checks if a collection is empty.
#[command]
pub async fn is_empty<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    name: String,
) -> std::result::Result<bool, CommandError> {
    state
        .with_db(|db| {
            let coll = db
                .get_collection(&name)
                .ok_or_else(|| Error::CollectionNotFound(name.clone()))?;
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
            let coll = db
                .get_collection(&name)
                .ok_or_else(|| Error::CollectionNotFound(name.clone()))?;
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
    let filter = request.filter.clone();

    let results = state
        .with_db(|db| {
            let coll = db
                .get_collection(&request.collection)
                .ok_or_else(|| Error::CollectionNotFound(request.collection.clone()))?;

            // Convert vectors to slices
            let vector_refs: Vec<&[f32]> = request.vectors.iter().map(Vec::as_slice).collect();

            let parsed_filter: Option<velesdb_core::Filter> = if let Some(ref filter_json) = filter
            {
                Some(
                    serde_json::from_value(filter_json.clone())
                        .map_err(|e| Error::InvalidConfig(format!("Invalid filter: {e}")))?,
                )
            } else {
                None
            };

            let search_results = coll.multi_query_search(
                &vector_refs,
                request.top_k,
                fusion_strategy,
                parsed_filter.as_ref(),
            )?;

            Ok(search_results
                .into_iter()
                .map(|r| SearchResult {
                    id: r.point.id,
                    score: r.score,
                    payload: r.point.payload,
                })
                .collect::<Vec<_>>())
        })
        .map_err(CommandError::from)?;

    Ok(SearchResponse {
        results,
        timing_ms: start.elapsed().as_secs_f64() * 1000.0,
    })
}

// ============================================================================
// AgentMemory Commands (EPIC-016 US-003)
// ============================================================================

use crate::types::{SemanticQueryRequest, SemanticQueryResult, SemanticStoreRequest};
use velesdb_core::agent::SemanticMemory;

/// Stores a knowledge fact in semantic memory.
#[command]
pub async fn semantic_store<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: SemanticStoreRequest,
) -> std::result::Result<(), CommandError> {
    state
        .with_db(|db| {
            let memory = SemanticMemory::new_from_db(db, request.embedding.len())
                .map_err(|e| Error::InvalidConfig(e.to_string()))?;
            memory
                .store(request.id, &request.content, &request.embedding)
                .map_err(|e| Error::InvalidConfig(e.to_string()))?;
            Ok(())
        })
        .map_err(CommandError::from)
}

/// Queries semantic memory by similarity search.
#[command]
pub async fn semantic_query<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: SemanticQueryRequest,
) -> std::result::Result<Vec<SemanticQueryResult>, CommandError> {
    state
        .with_db(|db| {
            let memory = SemanticMemory::new_from_db(db, request.embedding.len())
                .map_err(|e| Error::InvalidConfig(e.to_string()))?;
            let results = memory
                .query(&request.embedding, request.top_k)
                .map_err(|e| Error::InvalidConfig(e.to_string()))?;
            Ok(results
                .into_iter()
                .map(|(id, score, content)| SemanticQueryResult { id, score, content })
                .collect())
        })
        .map_err(CommandError::from)
}

// NOTE: Knowledge Graph Commands moved to commands_graph.rs (EPIC-061/US-008 refactoring)
// Re-export graph commands for backwards compatibility
pub use crate::commands_graph::{add_edge, get_edges, get_node_degree, traverse_graph};

#[cfg(test)]
#[path = "commands_tests.rs"]
mod tests;
