//! Tauri commands for `VelesDB` operations exposed via IPC.
#![allow(clippy::missing_errors_doc)]

use crate::error::CommandError;
use crate::events::emit_collection_updated;
use crate::helpers::{
    map_core_results, metric_to_string, parse_filter, parse_fusion_strategy, parse_metric,
    parse_storage_mode, require_collection, storage_mode_to_string, timed_search_response,
};
#[cfg(feature = "persistence")]
use crate::helpers::{parse_search_quality, require_vector_collection};
use crate::state::VelesDbState;
#[path = "commands_batch.rs"]
mod batch;
pub use crate::types::{
    default_fusion, default_metric, default_storage_mode, default_top_k, default_vector_weight,
};
use crate::types::{
    BatchSearchRequest, CollectionInfo, CreateCollectionRequest, CreateMetadataCollectionRequest,
    DeletePointsRequest, GetPointsRequest, GuardrailLimits, HybridSearchRequest, IdScoreOutput,
    MultiQuerySearchRequest, PointOutput, ScrollRequest, ScrollResponse, SearchRequest,
    SearchResponse, TextSearchRequest, TrainPqRequest, UpsertMetadataRequest, UpsertRequest,
};
#[cfg(feature = "persistence")]
use crate::types::{EnableStreamingRequest, StreamInsertRequest};
pub use batch::*;
use tauri::{command, AppHandle, Runtime, State};

/// The HNSW parameters the request pins; core resolves the rest (`[hnsw]`,
/// then the auto-tuning for the collection's dimension) and validates them.
pub(crate) const fn hnsw_overrides(
    request: &CreateCollectionRequest,
) -> velesdb_core::HnswOverrides {
    velesdb_core::HnswOverrides {
        m: request.hnsw_m,
        ef_construction: request.hnsw_ef_construction,
        max_elements: request.hnsw_max_elements,
        alpha: request.hnsw_alpha,
    }
}

/// Creates a new collection.
#[command]
pub async fn create_collection<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: CreateCollectionRequest,
) -> std::result::Result<CollectionInfo, CommandError> {
    let metric = parse_metric(&request.metric).map_err(CommandError::from)?;
    let storage_mode = parse_storage_mode(&request.storage_mode).map_err(CommandError::from)?;

    let result = state
        .run_db(move |db| {
            db.create_vector_collection_with_overrides(
                &request.name,
                request.dimension,
                metric,
                storage_mode,
                hnsw_overrides(&request),
                request.pq_rescore_oversampling,
            )?;
            Ok(CollectionInfo {
                name: request.name.clone(),
                dimension: request.dimension,
                metric: metric_to_string(metric).to_string(),
                count: 0,
                storage_mode: storage_mode_to_string(storage_mode).to_string(),
            })
        })
        .await
        .map_err(CommandError::from)?;

    Ok(result)
}

/// Creates a metadata-only collection (no vectors, just payloads).
#[command]
pub async fn create_metadata_collection<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: CreateMetadataCollectionRequest,
) -> std::result::Result<CollectionInfo, CommandError> {
    let result = state
        .run_db(move |db| {
            db.create_metadata_collection(&request.name)?;
            Ok(CollectionInfo {
                name: request.name,
                dimension: 0,
                metric: "none".to_string(),
                count: 0,
                storage_mode: "metadata_only".to_string(),
            })
        })
        .await
        .map_err(CommandError::from)?;

    Ok(result)
}

/// Deletes a collection.
#[command]
pub async fn delete_collection<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    name: String,
) -> std::result::Result<(), CommandError> {
    state
        .run_db(move |db| {
            db.delete_collection(&name)?;
            Ok(())
        })
        .await
        .map_err(CommandError::from)?;

    Ok(())
}

/// Lists all collections.
#[command]
pub async fn list_collections<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
) -> std::result::Result<Vec<CollectionInfo>, CommandError> {
    state
        .run_db(move |db| {
            let names = db.list_collections();
            let mut collections = Vec::new();
            for name in names {
                // Each entry carries the collection's count — a read the gate
                // decides per collection. A denied (or scoped, which a count
                // cannot honour) collection contributes no entry.
                let decided = crate::helpers::authorize_collection_read(
                    &db,
                    &name,
                    velesdb_core::observer::QueryOperationKind::Select,
                );
                if decided.map(|scope| scope.is_none()).unwrap_or(false) {
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
            }
            Ok(collections)
        })
        .await
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
        .run_db(move |db| {
            crate::helpers::require_unscoped_read(
                &db,
                &name,
                velesdb_core::observer::QueryOperationKind::Select,
                "collection metadata reads",
            )?;
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
        .await
        .map_err(CommandError::from)
}

/// Looks up the collection, builds points via `build`, inserts them via
/// `insert`, then emits a collection-updated event tagged with `event`.
///
/// Shared skeleton of [`upsert`] and [`upsert_metadata`]: only the per-point
/// construction and the core insert method differ between callers.
async fn upsert_points<R, B, I>(
    app: &AppHandle<R>,
    state: &State<'_, VelesDbState>,
    collection: String,
    event: &str,
    build: B,
    insert: I,
) -> std::result::Result<usize, CommandError>
where
    R: Runtime,
    B: FnOnce() -> Vec<velesdb_core::Point> + Send + 'static,
    I: FnOnce(
            &velesdb_core::VectorCollection,
            Vec<velesdb_core::Point>,
        ) -> crate::error::Result<()>
        + Send
        + 'static,
{
    let coll_name = collection.clone();
    let count = state
        .run_db(move |db| {
            let coll = require_collection(&db, &coll_name)?;

            let points = build();
            let count = points.len();
            insert(&coll, points)?;
            Ok(count)
        })
        .await
        .map_err(CommandError::from)?;

    emit_collection_updated(app, &collection, event, count);
    Ok(count)
}

/// Upserts points into a collection.
#[command]
pub async fn upsert<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: UpsertRequest,
) -> std::result::Result<usize, CommandError> {
    let collection_name = request.collection.clone();
    upsert_points(
        &app,
        &state,
        collection_name,
        "upsert",
        move || {
            request
                .points
                .into_iter()
                .map(|p| velesdb_core::Point::new(p.id, p.vector, p.payload))
                .collect()
        },
        |coll, points| coll.upsert(points).map_err(crate::error::Error::Database),
    )
    .await
}

/// Upserts metadata-only points into a collection.
#[command]
pub async fn upsert_metadata<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: UpsertMetadataRequest,
) -> std::result::Result<usize, CommandError> {
    let collection_name = request.collection.clone();
    upsert_points(
        &app,
        &state,
        collection_name,
        "upsert_metadata",
        move || {
            request
                .points
                .into_iter()
                .map(|p| velesdb_core::Point::new(p.id, vec![], Some(p.payload)))
                .collect()
        },
        |coll, points| {
            coll.upsert_metadata(points)
                .map_err(crate::error::Error::Database)
        },
    )
    .await
}

/// Gets points by their IDs.
#[command]
pub async fn get_points<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: GetPointsRequest,
) -> std::result::Result<Vec<Option<PointOutput>>, CommandError> {
    state
        .run_db(move |db| {
            let coll = require_collection(&db, &request.collection)?;

            let scope = crate::helpers::authorize_collection_read(
                &db,
                &request.collection,
                velesdb_core::observer::QueryOperationKind::Select,
            )?;
            let points = coll.get(&request.ids);
            Ok(points
                .into_iter()
                .map(|opt| {
                    opt.filter(|p| {
                        scope
                            .as_ref()
                            .is_none_or(|f| f.matches_point(p.id, p.payload.as_ref()))
                    })
                    .map(|p| PointOutput {
                        id: p.id,
                        vector: p.vector,
                        payload: p.payload,
                    })
                })
                .collect())
        })
        .await
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
        .run_db(move |db| {
            let coll = require_collection(&db, &request.collection)?;

            coll.delete(&request.ids)?;
            Ok(())
        })
        .await
        .map_err(CommandError::from)
}

/// Parsed quality argument: a real [`velesdb_core::SearchQuality`] with
/// `persistence`, otherwise the unit type (quality unsupported).
#[cfg(feature = "persistence")]
type QualityArg<'a> = Option<&'a velesdb_core::SearchQuality>;
#[cfg(not(feature = "persistence"))]
type QualityArg<'a> = Option<&'a ()>;

/// Core's search quality for a parsed `quality` field (`None` without the
/// `persistence` feature, where the field is refused before this runs).
#[cfg(feature = "persistence")]
fn core_quality(quality: QualityArg<'_>) -> Option<velesdb_core::SearchQuality> {
    quality.copied()
}

/// Core's search quality for a parsed `quality` field (`None` without the
/// `persistence` feature, where the field is refused before this runs).
#[cfg(not(feature = "persistence"))]
fn core_quality(_quality: QualityArg<'_>) -> Option<velesdb_core::SearchQuality> {
    None
}

/// Searches for similar vectors.
///
/// Supports an optional `quality` mode (e.g. "fast", "balanced", "accurate",
/// "perfect", "auto", "custom:\<ef\>", "adaptive:\<min\>:\<max\>"),
/// honoured with or without a filter.
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
    crate::helpers::reject_unsupported_quality(request.quality.as_ref())
        .map_err(CommandError::from)?;

    let results = state
        .run_db(move |db| {
            require_collection(&db, &request.collection)?;
            let search_results = db.gated_search(
                &request.collection,
                None,
                None,
                velesdb_core::GatedRead::Dense {
                    query: &request.vector,
                    k: request.top_k,
                    ef: None,
                    #[cfg(feature = "persistence")]
                    quality: parsed_quality,
                    #[cfg(not(feature = "persistence"))]
                    quality: None,
                    filter: parsed_filter.as_ref(),
                },
            )?;
            Ok(map_core_results(search_results))
        })
        .await
        .map_err(CommandError::from)?;

    Ok(timed_search_response(results, start))
}

/// Projects full search results down to ID + score pairs.
fn project_search_ids(results: Vec<velesdb_core::SearchResult>) -> Vec<IdScoreOutput> {
    results
        .into_iter()
        .map(|r| IdScoreOutput {
            id: r.point.id,
            score: r.score,
        })
        .collect()
}

/// k-NN search returning only IDs and scores (no payloads).
///
/// Uses the optimized core `search_ids` path when neither a filter nor a
/// `quality` mode is supplied, which skips payload hydration. Otherwise runs
/// core's `search_dense`, which honours a filter and a quality together, and
/// projects the results to IDs + scores.
#[command]
pub async fn search_ids<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: SearchRequest,
) -> std::result::Result<Vec<IdScoreOutput>, CommandError> {
    let parsed_filter = parse_filter(&request.filter).map_err(CommandError::from)?;
    #[cfg(feature = "persistence")]
    let parsed_quality = parse_search_quality(&request.quality).map_err(CommandError::from)?;
    #[cfg(not(feature = "persistence"))]
    let parsed_quality: Option<()> = {
        crate::helpers::reject_unsupported_quality(request.quality.as_ref())
            .map_err(CommandError::from)?;
        None
    };

    state
        .run_db(move |db| {
            let coll = require_collection(&db, &request.collection)?;
            crate::helpers::run_gated_search(
                &db,
                &request.collection,
                velesdb_core::observer::QueryOperationKind::VectorSearch,
                |scope| {
                    let parsed_filter = crate::helpers::and_scope(parsed_filter, scope);
                    // Optimized id-only path only when no filter and no quality; both
                    // require the full search that hydrates results before projecting.
                    let scored: Vec<IdScoreOutput> =
                        if parsed_filter.is_none() && parsed_quality.is_none() {
                            coll.search_ids(&request.vector, request.top_k)?
                                .into_iter()
                                .map(|r| IdScoreOutput {
                                    id: r.id,
                                    score: r.score,
                                })
                                .collect()
                        } else {
                            project_search_ids(coll.search_dense(
                                &request.vector,
                                request.top_k,
                                None,
                                core_quality(parsed_quality.as_ref()),
                                parsed_filter.as_ref(),
                            )?)
                        };
                    Ok(scored)
                },
            )
        })
        .await
        .map_err(CommandError::from)
}

#[command]
pub async fn text_search<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: TextSearchRequest,
) -> std::result::Result<SearchResponse, CommandError> {
    let parsed_filter = parse_filter(&request.filter).map_err(CommandError::from)?;
    let collection = request.collection.clone();

    let operation = velesdb_core::observer::QueryOperationKind::TextSearch;
    batch::run_filtered_search(
        &state,
        collection,
        operation,
        parsed_filter,
        move |coll, filter| {
            match filter {
                Some(f) => coll.text_search_with_filter(&request.query, request.top_k, f),
                None => coll.text_search(&request.query, request.top_k),
            }
            .map_err(crate::error::Error::Database)
        },
    )
    .await
}

/// Hybrid search combining vector similarity and BM25.
#[command]
pub async fn hybrid_search<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: HybridSearchRequest,
) -> std::result::Result<SearchResponse, CommandError> {
    let parsed_filter = parse_filter(&request.filter).map_err(CommandError::from)?;
    let collection = request.collection.clone();

    let operation = velesdb_core::observer::QueryOperationKind::HybridSearch;
    batch::run_filtered_search(
        &state,
        collection,
        operation,
        parsed_filter,
        move |coll, filter| {
            match filter {
                Some(f) => coll.hybrid_search_with_filter(
                    &request.vector,
                    &request.query,
                    request.top_k,
                    Some(request.vector_weight),
                    f,
                ),
                None => coll.hybrid_search(
                    &request.vector,
                    &request.query,
                    request.top_k,
                    Some(request.vector_weight),
                ),
            }
            .map_err(crate::error::Error::Database)
        },
    )
    .await
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
        .run_db(move |db| {
            crate::helpers::require_unscoped_read(
                &db,
                &name,
                velesdb_core::observer::QueryOperationKind::Select,
                "is_empty",
            )?;
            let coll = require_collection(&db, &name)?;
            Ok(coll.is_empty())
        })
        .await
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
        .run_db(move |db| {
            let coll = require_collection(&db, &name)?;
            coll.flush()?;
            Ok(())
        })
        .await
        .map_err(CommandError::from)
}

/// Compacts on-disk storage for a collection, reclaiming space from deleted
/// vectors. Returns the number of bytes reclaimed.
#[command]
pub async fn compact_storage<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    name: String,
) -> std::result::Result<u64, CommandError> {
    state
        .run_db(move |db| {
            let coll = require_collection(&db, &name)?;
            let freed = coll.compact_storage()?;
            Ok(u64::try_from(freed).unwrap_or(u64::MAX))
        })
        .await
        .map_err(CommandError::from)
}

/// Updates query guardrail limits for every collection in the database.
///
/// This is a full replacement: all fields of `limits` are applied, once they
/// pass core's `QueryLimits::validate` (a `rate_limit_qps` or
/// `circuit_failure_threshold` of `0` is refused with `VELES-009` and nothing
/// changes).
#[command]
pub async fn update_guardrails<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    limits: GuardrailLimits,
) -> std::result::Result<(), CommandError> {
    state
        .run_db(move |db| {
            let limits: velesdb_core::guardrails::QueryLimits = limits.into();
            db.update_guardrails_with(|live| live.clone_from(&limits))?;
            Ok(())
        })
        .await
        .map_err(CommandError::from)
}

/// Returns the current query guardrail limits for a collection.
#[command]
pub async fn get_guardrails<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    name: String,
) -> std::result::Result<GuardrailLimits, CommandError> {
    state
        .run_db(move |db| {
            let coll = require_collection(&db, &name)?;
            Ok(GuardrailLimits::from(coll.guard_rails().limits()))
        })
        .await
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
        .run_db(move |db| {
            let coll = require_collection(&db, &request.collection)?;
            let scope = crate::helpers::authorize_collection_read(
                &db,
                &request.collection,
                velesdb_core::observer::QueryOperationKind::Select,
            )?;
            let parsed_filter = crate::helpers::and_scope(parsed_filter, scope);
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
        .await
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
        .run_db(move |db| {
            let coll = require_collection(&db, &request.collection)?;
            crate::helpers::run_gated_search(
                &db,
                &request.collection,
                velesdb_core::observer::QueryOperationKind::VectorSearch,
                |scope| {
                    let parsed_filter = crate::helpers::and_scope(parsed_filter, scope);

                    let vector_refs: Vec<&[f32]> =
                        request.vectors.iter().map(Vec::as_slice).collect();

                    let search_results = coll.multi_query_search(
                        &vector_refs,
                        request.top_k,
                        fusion_strategy,
                        parsed_filter.as_ref(),
                    )?;

                    Ok(map_core_results(search_results))
                },
            )
        })
        .await
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
        .run_db(move |db| {
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
            db.execute_query(&query, &empty_params)?;

            Ok("PQ training complete".to_string())
        })
        .await
        .map_err(CommandError::from)
}

// ============================================================================
// Streaming Insert Command
// ============================================================================

/// Stream-inserts points into a collection through its streaming pipeline.
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
        .run_db(move |db| {
            let coll = require_vector_collection(&db, &request.collection)?;

            let points: Vec<_> = request
                .points
                .into_iter()
                .map(|p| velesdb_core::Point::new(p.id, p.vector, p.payload))
                .collect();
            // One batch, checked before anything is queued: a point an upsert
            // would refuse fails the call with core's code instead of being
            // dropped later by the drain.
            coll.validate_points_for_upsert(&points)?;
            coll.stream_insert_batch(points)
                .map_err(crate::error::Error::from)
        })
        .await
        .map_err(CommandError::from)?;

    emit_collection_updated(&app, &collection_name, "stream_insert", count);
    Ok(count)
}

/// Builds a core [`StreamingConfig`](velesdb_core::StreamingConfig) from the
/// request, falling back to engine defaults for any omitted field.
#[cfg(feature = "persistence")]
pub(crate) fn streaming_config_from_request(
    request: &EnableStreamingRequest,
) -> velesdb_core::StreamingConfig {
    let base = velesdb_core::StreamingConfig::default();
    velesdb_core::StreamingConfig::new(
        request.buffer_size.unwrap_or(base.buffer_size),
        request.batch_size.unwrap_or(base.batch_size),
        request.flush_interval_ms.unwrap_or(base.flush_interval_ms),
    )
}

/// Enables streaming ingestion on a collection.
///
/// Spawns the background drain task so subsequent `stream_insert` calls accept
/// points instead of returning a "not configured" backpressure error. Requires
/// the `persistence` feature.
#[cfg(feature = "persistence")]
#[command]
pub async fn enable_streaming<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: EnableStreamingRequest,
) -> std::result::Result<(), CommandError> {
    let collection_name = request.collection.clone();
    let config = streaming_config_from_request(&request);
    state
        .run_db(move |db| {
            let coll = require_vector_collection(&db, &request.collection)?;
            coll.enable_streaming(config)?;
            Ok(())
        })
        .await
        .map_err(CommandError::from)?;

    emit_collection_updated(&app, &collection_name, "enable_streaming", 0);
    Ok(())
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
