//! Collection management handlers.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use std::sync::Arc;

use crate::types::{CollectionResponse, CreateCollectionRequest, ErrorResponse};
use crate::AppState;
use velesdb_core::index::HnswParams;
use velesdb_core::{DistanceMetric, StorageMode};

use super::helpers::{core_error_response, error_response, get_collection_or_404};

/// List all collections.
#[utoipa::path(
    get,
    path = "/collections",
    tag = "collections",
    responses(
        (status = 200, description = "List of collections", body = Object)
    )
)]
pub async fn list_collections(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let collections = state.db.list_collections();
    Json(serde_json::json!({ "collections": collections }))
}

/// Create a new collection.
#[utoipa::path(
    post,
    path = "/collections",
    tag = "collections",
    request_body = CreateCollectionRequest,
    responses(
        (status = 201, description = "Collection created", body = Object),
        (status = 400, description = "Invalid request", body = ErrorResponse)
    )
)]
pub async fn create_collection(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateCollectionRequest>,
) -> impl IntoResponse {
    let metric = match parse_distance_metric(&req.metric) {
        Ok(m) => m,
        Err(resp) => return resp,
    };

    let storage_mode = match parse_storage_mode(&req.storage_mode) {
        Ok(s) => s,
        Err(resp) => return resp,
    };

    let result = match dispatch_create(&state, &req, metric, storage_mode) {
        Ok(r) => r,
        Err(resp) => return resp,
    };

    match result {
        Ok(()) => create_collection_success_response(&req),
        Err(e) => core_error_response(StatusCode::BAD_REQUEST, &e),
    }
}

/// Parse a distance metric string into the core enum.
///
/// Delegates to [`DistanceMetric::from_str`] to keep alias parsing in one place.
#[allow(clippy::result_large_err)]
fn parse_distance_metric(raw: &str) -> Result<DistanceMetric, axum::response::Response> {
    raw.parse::<DistanceMetric>()
        .map_err(|e| error_response(StatusCode::BAD_REQUEST, e.to_string()))
}

/// Parse a storage mode string into the core enum.
///
/// Delegates to [`StorageMode::from_str`] (single source of truth in `velesdb-core`).
#[allow(clippy::result_large_err)]
fn parse_storage_mode(raw: &str) -> Result<StorageMode, axum::response::Response> {
    raw.parse::<StorageMode>()
        .map_err(|e| error_response(StatusCode::BAD_REQUEST, e))
}

/// Build a full `HnswParams` override from the request fields, or return
/// `None` when the caller supplied no HNSW tuning fields at all.
///
/// The base parameters come from `HnswParams::auto(dimension)` so that
/// unspecified fields inherit the engine's dimension-aware defaults.
/// `storage_mode` always mirrors the top-level collection `storage_mode`
/// — callers cannot desync the HNSW inner storage mode from the
/// collection's advertised quantisation (the `HnswParams::storage_mode`
/// field is a denormalised copy that the engine keeps in sync).
fn build_hnsw_params_override(
    req: &CreateCollectionRequest,
    dimension: usize,
    storage_mode: StorageMode,
) -> Option<HnswParams> {
    if req.hnsw_m.is_none()
        && req.hnsw_ef_construction.is_none()
        && req.hnsw_alpha.is_none()
        && req.hnsw_max_elements.is_none()
    {
        return None;
    }
    let base = HnswParams::auto(dimension);
    Some(HnswParams {
        max_connections: req.hnsw_m.unwrap_or(base.max_connections),
        ef_construction: req.hnsw_ef_construction.unwrap_or(base.ef_construction),
        max_elements: req.hnsw_max_elements.unwrap_or(base.max_elements),
        storage_mode,
        alpha: req.hnsw_alpha.unwrap_or(base.alpha),
    })
}

/// Create a vector collection, requiring a dimension in the request.
///
/// Applies advanced configuration overrides (pq_rescore_oversampling,
/// deferred_indexing, async_index_builder) in a second pass via
/// `VectorCollection::apply_advanced_config` once the base collection
/// has been registered. This two-step approach keeps the core
/// `Database::create_vector_collection_*` API stable while still
/// honouring the full PROP-CONFIG-ADVANCED field set on the REST
/// surface.
#[allow(clippy::result_large_err)]
fn create_vector_collection(
    state: &AppState,
    req: &CreateCollectionRequest,
    metric: DistanceMetric,
    storage_mode: StorageMode,
) -> Result<velesdb_core::error::Result<()>, axum::response::Response> {
    let dimension = req.dimension.ok_or_else(|| {
        error_response(
            StatusCode::BAD_REQUEST,
            "dimension is required for vector collections".to_string(),
        )
    })?;

    // Parse the advanced override fields up-front so a malformed JSON
    // payload fails the request before any collection is created on
    // disk. This avoids the half-initialised state where the base
    // collection exists but the advanced fields are missing.
    let advanced = parse_advanced_config(req)?;

    // Phase 1: create the base collection with HNSW params.
    //
    // Any of `hnsw_m`, `hnsw_ef_construction`, `hnsw_alpha`, or
    // `hnsw_max_elements` being present triggers the "with_params"
    // path so the caller-supplied values flow into a full `HnswParams`
    // starting from the engine's dimension-aware auto defaults. The
    // legacy `with_hnsw` helper cannot carry alpha/max_elements and
    // would silently drop them, re-introducing the PROP-HNSW-ALPHA gap.
    let base_result = if let Some(hnsw_params) =
        build_hnsw_params_override(req, dimension, storage_mode)
    {
        state.db.create_vector_collection_with_params(
            &req.name,
            dimension,
            metric,
            storage_mode,
            hnsw_params,
            None,
        )
    } else {
        state
            .db
            .create_vector_collection_with_options(&req.name, dimension, metric, storage_mode)
    };
    if let Err(e) = base_result {
        return Ok(Err(e));
    }

    // Phase 2: persist advanced overrides if any were requested.
    //
    // If this phase fails, we MUST roll back the Phase 1 collection
    // creation so callers do not end up with a half-initialised
    // collection on disk that would subsequently fail every retry
    // with `CollectionExists`. Any delete error during rollback is
    // logged but we still surface the original Phase 2 error to the
    // caller because it is more actionable.
    if advanced.has_any() {
        let Some(coll) = state.db.get_vector_collection(&req.name) else {
            return Ok(Err(velesdb_core::error::Error::CollectionNotFound(
                req.name.clone(),
            )));
        };
        if let Err(phase_two_err) = coll.apply_advanced_config(
            advanced.pq_rescore_oversampling,
            #[cfg(feature = "persistence")]
            advanced.deferred_indexing,
            advanced.async_index_builder,
        ) {
            // Drop the coll handle before the rollback to release any
            // read lock the registry hands back by default.
            drop(coll);
            let rollback_outcome = state.db.delete_collection(&req.name);
            if let Err(ref rollback_err) = rollback_outcome {
                tracing::warn!(
                    collection = %req.name,
                    rollback_error = %rollback_err,
                    phase_two_error = %phase_two_err,
                    "failed to roll back collection after apply_advanced_config error"
                );
            }

            // Post-rollback validation (S2-NEW-13, audit A P1 +
            // Devin ANALYSIS_0002 on PR #582): double-check that the
            // collection has actually been removed from the registry
            // before returning the Phase 2 error to the caller. If
            // the rollback failed AND the collection is still
            // present, the client retry will hit `CollectionExists`
            // and have no idea why — log a critical diagnostic so
            // operators can manually reconcile the orphaned state.
            // The original Phase 2 error is still the return value
            // because it is more actionable for the caller.
            if state.db.get_any_collection(&req.name).is_some() {
                tracing::error!(
                    collection = %req.name,
                    rollback_outcome = ?rollback_outcome,
                    phase_two_error = %phase_two_err,
                    "post-rollback invariant violated: collection still present in \
                     registry after delete_collection was attempted. Manual \
                     reconciliation required — client retries will fail with \
                     CollectionExists until the orphaned collection is cleaned up."
                );
            }

            return Ok(Err(phase_two_err));
        }
    }

    Ok(Ok(()))
}

/// Parsed advanced override fields for the create-collection pipeline.
///
/// The outer `Option` signals whether the field was present in the
/// request body; the inner `Option` carries the value the caller
/// wanted to persist (including explicit `null` → `Some(None)`).
/// A local clippy allow is applied because the three-state semantics
/// are the intended contract here.
#[allow(clippy::option_option)]
#[derive(Default)]
struct AdvancedCreateOverrides {
    pq_rescore_oversampling: Option<Option<u32>>,
    #[cfg(feature = "persistence")]
    deferred_indexing: Option<Option<velesdb_core::collection::streaming::DeferredIndexerConfig>>,
    async_index_builder:
        Option<Option<velesdb_core::collection::streaming::AsyncIndexBuilderConfig>>,
}

impl AdvancedCreateOverrides {
    #[cfg(feature = "persistence")]
    fn has_any(&self) -> bool {
        self.pq_rescore_oversampling.is_some()
            || self.deferred_indexing.is_some()
            || self.async_index_builder.is_some()
    }

    #[cfg(not(feature = "persistence"))]
    fn has_any(&self) -> bool {
        self.pq_rescore_oversampling.is_some() || self.async_index_builder.is_some()
    }
}

/// Parses the advanced override JSON fields on `CreateCollectionRequest`
/// into typed `CollectionConfig` fragments. A malformed JSON payload
/// becomes a 400 response.
#[allow(clippy::result_large_err)]
fn parse_advanced_config(
    req: &CreateCollectionRequest,
) -> Result<AdvancedCreateOverrides, axum::response::Response> {
    let mut overrides = AdvancedCreateOverrides {
        pq_rescore_oversampling: req.pq_rescore_oversampling.map(Some),
        ..Default::default()
    };

    #[cfg(feature = "persistence")]
    if let Some(ref value) = req.deferred_indexing {
        let parsed: velesdb_core::collection::streaming::DeferredIndexerConfig =
            serde_json::from_value(value.clone()).map_err(|e| {
                error_response(
                    StatusCode::BAD_REQUEST,
                    format!("Invalid 'deferred_indexing' configuration: {e}"),
                )
            })?;
        overrides.deferred_indexing = Some(Some(parsed));
    }

    if let Some(ref value) = req.async_index_builder {
        let parsed: velesdb_core::collection::streaming::AsyncIndexBuilderConfig =
            serde_json::from_value(value.clone()).map_err(|e| {
                error_response(
                    StatusCode::BAD_REQUEST,
                    format!("Invalid 'async_index_builder' configuration: {e}"),
                )
            })?;
        overrides.async_index_builder = Some(Some(parsed));
    }

    Ok(overrides)
}

/// Parses the optional `graph_schema` JSON field on
/// `CreateCollectionRequest` into a typed `GraphSchema`. When the field
/// is absent the schemaless default is returned, preserving backward
/// compatibility with callers that relied on the previous behaviour.
#[allow(clippy::result_large_err)]
fn parse_graph_schema(
    req: &CreateCollectionRequest,
) -> Result<velesdb_core::GraphSchema, axum::response::Response> {
    match req.graph_schema.as_ref() {
        Some(value) => serde_json::from_value(value.clone()).map_err(|e| {
            error_response(
                StatusCode::BAD_REQUEST,
                format!("Invalid 'graph_schema' payload: {e}"),
            )
        }),
        None => Ok(velesdb_core::GraphSchema::schemaless()),
    }
}

/// Dispatch collection creation based on `collection_type`.
#[allow(clippy::result_large_err)]
fn dispatch_create(
    state: &AppState,
    req: &CreateCollectionRequest,
    metric: DistanceMetric,
    storage_mode: StorageMode,
) -> Result<velesdb_core::error::Result<()>, axum::response::Response> {
    match req.collection_type.to_lowercase().as_str() {
        "metadata_only" | "metadata-only" | "metadata" => {
            Ok(state.db.create_metadata_collection(&req.name))
        }
        "graph" | "knowledge_graph" | "kg" => {
            let schema = parse_graph_schema(req)?;
            Ok(state.db.create_graph_collection(&req.name, schema))
        }
        "vector" | "" => create_vector_collection(state, req, metric, storage_mode),
        _ => Err(error_response(
            StatusCode::BAD_REQUEST,
            format!(
                "Invalid collection_type: {}. Valid: vector, graph, metadata_only",
                req.collection_type
            ),
        )),
    }
}

/// Build a 201 Created response for successful collection creation.
fn create_collection_success_response(req: &CreateCollectionRequest) -> axum::response::Response {
    let mut warnings = Vec::new();
    let is_vector = matches!(req.collection_type.to_lowercase().as_str(), "vector" | "");
    if is_vector {
        warnings.push("Collection dimension and metric are immutable after creation. If your embedding model changes, create a new collection and reindex data.");
        warnings.push("For first queries, start without strict filters/thresholds, then tighten progressively.");
    }

    (
        StatusCode::CREATED,
        Json(serde_json::json!({
            "message": "Collection created",
            "name": req.name,
            "type": req.collection_type,
            "warnings": warnings
        })),
    )
        .into_response()
}

/// Get collection information.
#[utoipa::path(
    get,
    path = "/collections/{name}",
    tag = "collections",
    params(
        ("name" = String, Path, description = "Collection name")
    ),
    responses(
        (status = 200, description = "Collection details", body = CollectionResponse),
        (status = 404, description = "Collection not found", body = ErrorResponse)
    )
)]
pub async fn get_collection(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let collection = match get_collection_or_404(&state, &name) {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    let config = collection.config();
    Json(CollectionResponse {
        name: config.name,
        dimension: config.dimension,
        metric: format!("{:?}", config.metric).to_lowercase(),
        point_count: config.point_count,
        storage_mode: format!("{:?}", config.storage_mode).to_lowercase(),
    })
    .into_response()
}

/// Run a quick sanity check for onboarding and troubleshooting.
#[utoipa::path(
    get,
    path = "/collections/{name}/sanity",
    tag = "collections",
    params(
        ("name" = String, Path, description = "Collection name")
    ),
    responses(
        (status = 200, description = "Collection sanity status", body = Object),
        (status = 404, description = "Collection not found", body = ErrorResponse)
    )
)]
pub async fn collection_sanity(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let collection = match get_collection_or_404(&state, &name) {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    let config = collection.config();
    build_sanity_response(&state, &config, &collection)
}

/// Build the JSON sanity check response body.
fn build_sanity_response(
    state: &AppState,
    config: &velesdb_core::collection::CollectionConfig,
    collection: &velesdb_core::AnyCollection,
) -> axum::response::Response {
    let has_data = config.point_count > 0;
    Json(serde_json::json!({
        "collection": config.name,
        "dimension": config.dimension,
        "metric": format!("{:?}", config.metric).to_lowercase(),
        "point_count": config.point_count,
        "is_empty": collection.is_empty(),
        "checks": {
            "has_vectors": has_data,
            "search_ready": has_data,
            "dimension_configured": config.dimension > 0
        },
        "diagnostics": {
            "search_requests_total": state.onboarding_metrics.search_requests_total.load(std::sync::atomic::Ordering::Relaxed),
            "dimension_mismatch_total": state.onboarding_metrics.dimension_mismatch_total.load(std::sync::atomic::Ordering::Relaxed),
            "empty_search_results_total": state.onboarding_metrics.empty_search_results_total.load(std::sync::atomic::Ordering::Relaxed),
            "filter_parse_errors_total": state.onboarding_metrics.filter_parse_errors_total.load(std::sync::atomic::Ordering::Relaxed)
        },
        "hints": if has_data {
            vec![
                "Run a search without strict filters first, then tighten filters progressively."
            ]
        } else {
            vec![
                "Insert at least one known vector before evaluating search quality.",
                "Verify you are querying the intended collection."
            ]
        }
    }))
    .into_response()
}

/// Delete a collection.
#[utoipa::path(
    delete,
    path = "/collections/{name}",
    tag = "collections",
    params(
        ("name" = String, Path, description = "Collection name")
    ),
    responses(
        (status = 200, description = "Collection deleted", body = Object),
        (status = 404, description = "Collection not found", body = ErrorResponse)
    )
)]
pub async fn delete_collection(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    match state.db.delete_collection(&name) {
        Ok(()) => Json(serde_json::json!({
            "message": "Collection deleted",
            "name": name
        }))
        .into_response(),
        Err(e) => core_error_response(StatusCode::NOT_FOUND, &e),
    }
}

/// Check if a collection is empty.
#[utoipa::path(
    get,
    path = "/collections/{name}/empty",
    tag = "collections",
    params(
        ("name" = String, Path, description = "Collection name")
    ),
    responses(
        (status = 200, description = "Empty status", body = Object),
        (status = 404, description = "Collection not found", body = ErrorResponse)
    )
)]
pub async fn is_empty(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let collection = match get_collection_or_404(&state, &name) {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    Json(serde_json::json!({
        "is_empty": collection.is_empty()
    }))
    .into_response()
}

/// Flush pending changes to disk.
#[utoipa::path(
    post,
    path = "/collections/{name}/flush",
    tag = "collections",
    params(
        ("name" = String, Path, description = "Collection name")
    ),
    responses(
        (status = 200, description = "Flushed successfully", body = Object),
        (status = 404, description = "Collection not found", body = ErrorResponse),
        (status = 500, description = "Flush failed", body = ErrorResponse)
    )
)]
pub async fn flush_collection(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let collection = match get_collection_or_404(&state, &name) {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    match collection.flush() {
        Ok(()) => Json(serde_json::json!({
            "message": "Flushed successfully",
            "collection": name
        }))
        .into_response(),
        Err(e) => core_error_response(StatusCode::INTERNAL_SERVER_ERROR, &e),
    }
}
