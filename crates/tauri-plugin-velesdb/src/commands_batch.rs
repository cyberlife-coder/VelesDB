//! The `batch_search` command — a `#[path]` child of `crate::commands` that
//! keeps `commands.rs` under the file budget. It reads the collection the
//! same way the sibling commands do: through the read gate, with the
//! observer's scope AND-composed into each per-search filter.
#![allow(clippy::missing_errors_doc)]

use super::dispatch_quality_search;
#[cfg(feature = "persistence")]
use super::parse_search_quality;
use super::BatchSearchRequest;
use super::{parse_filter, require_collection, CommandError, SearchResponse, VelesDbState};
use crate::helpers::{map_core_results, timed_search_response};
use tauri::{command, AppHandle, Runtime, State};

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
        batch_search_per_query(&state, request).await?
    } else {
        batch_search_bulk(&state, request).await?
    };

    let timing_ms = start.elapsed().as_secs_f64() * 1000.0;
    Ok(batch_results
        .into_iter()
        .map(|results| SearchResponse { results, timing_ms })
        .collect())
}

/// Per-query batch search with per-search quality dispatch.
async fn batch_search_per_query(
    state: &VelesDbState,
    request: BatchSearchRequest,
) -> std::result::Result<Vec<Vec<crate::types::SearchResult>>, CommandError> {
    state
        .run_db(move |db| {
            let coll = require_collection(&db, &request.collection)?;
            let scope = crate::helpers::authorize_collection_read(
                &db,
                &request.collection,
                velesdb_core::observer::QueryOperationKind::VectorSearch,
            )?;
            let mut all_results = Vec::with_capacity(request.searches.len());
            for s in &request.searches {
                let filter = crate::helpers::and_scope(parse_filter(&s.filter)?, scope.clone());
                #[cfg(feature = "persistence")]
                let quality = parse_search_quality(&s.quality)
                    .map_err(|e| crate::error::Error::InvalidConfig(e.to_string()))?;
                #[cfg(not(feature = "persistence"))]
                let quality: Option<()> = {
                    crate::helpers::reject_unsupported_quality(s.quality.as_ref())?;
                    None
                };

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
        .await
        .map_err(CommandError::from)
}

/// Optimized bulk batch search (no per-query quality).
async fn batch_search_bulk(
    state: &VelesDbState,
    request: BatchSearchRequest,
) -> std::result::Result<Vec<Vec<crate::types::SearchResult>>, CommandError> {
    state
        .run_db(move |db| {
            let coll = require_collection(&db, &request.collection)?;
            let scope = crate::helpers::authorize_collection_read(
                &db,
                &request.collection,
                velesdb_core::observer::QueryOperationKind::VectorSearch,
            )?;

            let query_refs: Vec<&[f32]> = request
                .searches
                .iter()
                .map(|s| s.vector.as_slice())
                .collect();
            let filters: Vec<Option<velesdb_core::Filter>> = request
                .searches
                .iter()
                .map(|s| {
                    parse_filter(&s.filter).map(|f| crate::helpers::and_scope(f, scope.clone()))
                })
                .collect::<crate::error::Result<Vec<_>>>()?;

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
        .await
        .map_err(CommandError::from)
}

/// Runs a filter-aware search on `collection` and wraps the mapped results in a
/// timed [`SearchResponse`].
///
/// Shared skeleton of [`text_search`] and [`hybrid_search`]: `search` receives
/// the collection plus the parsed filter (if any) and dispatches to the
/// matching core call. The whole search runs on the blocking pool via
/// [`VelesDbState::run_db`].
pub(super) async fn run_filtered_search<F>(
    state: &State<'_, VelesDbState>,
    collection: String,
    parsed_filter: Option<velesdb_core::Filter>,
    search: F,
) -> std::result::Result<SearchResponse, CommandError>
where
    F: FnOnce(
            &velesdb_core::VectorCollection,
            Option<&velesdb_core::Filter>,
        ) -> crate::error::Result<Vec<velesdb_core::SearchResult>>
        + Send
        + 'static,
{
    let start = std::time::Instant::now();
    let results = state
        .run_db(move |db| {
            let coll = require_collection(&db, &collection)?;
            let scope = crate::helpers::authorize_collection_read(
                &db,
                &collection,
                velesdb_core::observer::QueryOperationKind::Select,
            )?;
            let parsed_filter = crate::helpers::and_scope(parsed_filter, scope);
            let search_results = search(&coll, parsed_filter.as_ref())?;
            Ok(map_core_results(search_results))
        })
        .await
        .map_err(CommandError::from)?;

    Ok(timed_search_response(results, start))
}
