//! Tauri commands for `VelesDB` operations.
//!
//! These commands are exposed to the frontend via the Tauri IPC system.

#![allow(clippy::missing_errors_doc)] // Tauri commands have implicit error handling

use tauri::{command, AppHandle, Runtime, State};

use crate::error::{CommandError, Error};
use crate::helpers::{metric_to_string, parse_metric, parse_storage_mode, storage_mode_to_string};
use crate::state::VelesDbState;
use crate::types::{
    BatchSearchRequest, CollectionInfo, CreateCollectionRequest, CreateMetadataCollectionRequest,
    DeletePointsRequest, GetPointsRequest, HybridSearchRequest, MultiQuerySearchRequest,
    PointOutput, QueryRequest, SearchRequest, SearchResponse, SearchResult, TextSearchRequest,
    UpsertMetadataRequest, UpsertRequest,
};
// Re-export default functions for serde attributes in types that might still be in this file
pub use crate::types::{
    default_fusion, default_metric, default_storage_mode, default_top_k, default_vector_weight,
};

/// Creates a new collection.
#[command]
pub async fn create_collection<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: CreateCollectionRequest,
) -> std::result::Result<CollectionInfo, CommandError> {
    let metric = parse_metric(&request.metric).map_err(CommandError::from)?;

    let storage_mode = parse_storage_mode(&request.storage_mode).map_err(CommandError::from)?;

    state
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
        .map_err(CommandError::from)
}

/// Creates a metadata-only collection (no vectors, just payloads).
#[command]
pub async fn create_metadata_collection<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: CreateMetadataCollectionRequest,
) -> std::result::Result<CollectionInfo, CommandError> {
    state
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
        .map_err(CommandError::from)
}

/// Deletes a collection.
#[command]
pub async fn delete_collection<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    name: String,
) -> std::result::Result<(), CommandError> {
    state
        .with_db(|db| {
            db.delete_collection(&name)?;
            Ok(())
        })
        .map_err(CommandError::from)
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
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: UpsertRequest,
) -> std::result::Result<usize, CommandError> {
    state
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
        .map_err(CommandError::from)
}

/// Upserts metadata-only points into a collection.
#[command]
pub async fn upsert_metadata<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: UpsertMetadataRequest,
) -> std::result::Result<usize, CommandError> {
    state
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
        .map_err(CommandError::from)
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

/// Executes a `VelesQL` query.
#[command]
pub async fn query<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: QueryRequest,
) -> std::result::Result<SearchResponse, CommandError> {
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
#[allow(clippy::cast_possible_truncation)]
pub async fn multi_query_search<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: MultiQuerySearchRequest,
) -> std::result::Result<SearchResponse, CommandError> {
    use velesdb_core::fusion::FusionStrategy;

    let start = std::time::Instant::now();

    // Parse fusion strategy
    let fusion_strategy = match request.fusion.to_lowercase().as_str() {
        "rrf" => {
            let k = request
                .fusion_params
                .as_ref()
                .and_then(|p| p.get("k"))
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(60) as u32;
            FusionStrategy::RRF { k }
        }
        "average" => FusionStrategy::Average,
        "maximum" => FusionStrategy::Maximum,
        "weighted" => {
            let params = request.fusion_params.as_ref();
            let avg_weight = params
                .and_then(|p| p.get("avgWeight").or_else(|| p.get("avg_weight")))
                .and_then(serde_json::Value::as_f64)
                .unwrap_or(0.6) as f32;
            let max_weight = params
                .and_then(|p| p.get("maxWeight").or_else(|| p.get("max_weight")))
                .and_then(serde_json::Value::as_f64)
                .unwrap_or(0.3) as f32;
            let hit_weight = params
                .and_then(|p| p.get("hitWeight").or_else(|| p.get("hit_weight")))
                .and_then(serde_json::Value::as_f64)
                .unwrap_or(0.1) as f32;
            FusionStrategy::Weighted {
                avg_weight,
                max_weight,
                hit_weight,
            }
        }
        _ => FusionStrategy::RRF { k: 60 },
    };

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
// Tests (TDD - written BEFORE implementation verification)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_metric_cosine() {
        // Arrange & Act
        let result = parse_metric("cosine");

        // Assert
        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_metric_euclidean() {
        // Arrange & Act
        let result = parse_metric("euclidean");

        // Assert
        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_metric_l2_alias() {
        // Arrange & Act
        let result = parse_metric("l2");

        // Assert
        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_metric_dot() {
        // Arrange & Act
        let result = parse_metric("dot");

        // Assert
        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_metric_hamming() {
        // Arrange & Act
        let result = parse_metric("hamming");

        // Assert
        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_metric_jaccard() {
        // Arrange & Act
        let result = parse_metric("jaccard");

        // Assert
        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_metric_invalid() {
        // Arrange & Act
        let result = parse_metric("unknown");

        // Assert
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("Unknown metric"));
    }

    #[test]
    fn test_parse_metric_case_insensitive() {
        // Arrange & Act & Assert
        assert!(parse_metric("COSINE").is_ok());
        assert!(parse_metric("Euclidean").is_ok());
        assert!(parse_metric("DOT").is_ok());
    }

    #[test]
    fn test_metric_to_string() {
        use velesdb_core::distance::DistanceMetric;

        // Arrange & Act & Assert
        assert_eq!(metric_to_string(DistanceMetric::Cosine), "cosine");
        assert_eq!(metric_to_string(DistanceMetric::Euclidean), "euclidean");
        assert_eq!(metric_to_string(DistanceMetric::DotProduct), "dot");
        assert_eq!(metric_to_string(DistanceMetric::Hamming), "hamming");
        assert_eq!(metric_to_string(DistanceMetric::Jaccard), "jaccard");
    }

    #[test]
    fn test_default_metric() {
        // Act
        let metric = default_metric();

        // Assert
        assert_eq!(metric, "cosine");
    }

    #[test]
    fn test_default_top_k() {
        // Act
        let k = default_top_k();

        // Assert
        assert_eq!(k, 10);
    }

    #[test]
    fn test_default_vector_weight() {
        // Act
        let weight = default_vector_weight();

        // Assert
        assert!((weight - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn test_create_collection_request_deserialize() {
        // Arrange
        let json = r#"{"name": "test", "dimension": 768}"#;

        // Act
        let request: CreateCollectionRequest = serde_json::from_str(json).unwrap();

        // Assert
        assert_eq!(request.name, "test");
        assert_eq!(request.dimension, 768);
        assert_eq!(request.metric, "cosine"); // default
        assert_eq!(request.storage_mode, "full"); // default
    }

    #[test]
    fn test_search_request_deserialize() {
        // Arrange
        let json = r#"{"collection": "docs", "vector": [0.1, 0.2, 0.3]}"#;

        // Act
        let request: SearchRequest = serde_json::from_str(json).unwrap();

        // Assert
        assert_eq!(request.collection, "docs");
        assert_eq!(request.vector, vec![0.1, 0.2, 0.3]);
        assert_eq!(request.top_k, 10); // default
    }

    #[test]
    fn test_collection_info_serialize() {
        // Arrange
        let info = CollectionInfo {
            name: "test".to_string(),
            dimension: 768,
            metric: "cosine".to_string(),
            count: 100,
            storage_mode: "full".to_string(),
        };

        // Act
        let json = serde_json::to_string(&info).unwrap();

        // Assert
        assert!(json.contains("\"name\":\"test\""));
        assert!(json.contains("\"dimension\":768"));
        assert!(json.contains("\"metric\":\"cosine\""));
        assert!(json.contains("\"count\":100"));
        assert!(json.contains("\"storageMode\":\"full\""));
    }

    #[test]
    fn test_search_result_serialize() {
        // Arrange
        let result = SearchResult {
            id: 42,
            score: 0.95,
            payload: Some(serde_json::json!({"title": "Test"})),
        };

        // Act
        let json = serde_json::to_string(&result).unwrap();

        // Assert
        assert!(json.contains("\"id\":42"));
        assert!(json.contains("\"score\":0.95"));
        assert!(json.contains("\"title\":\"Test\""));
    }

    #[test]
    fn test_get_points_request_deserialize() {
        // Arrange
        let json = r#"{"collection": "docs", "ids": [1, 2, 3]}"#;

        // Act
        let request: GetPointsRequest = serde_json::from_str(json).unwrap();

        // Assert
        assert_eq!(request.collection, "docs");
        assert_eq!(request.ids, vec![1, 2, 3]);
    }

    #[test]
    fn test_delete_points_request_deserialize() {
        // Arrange
        let json = r#"{"collection": "docs", "ids": [1, 2]}"#;

        // Act
        let request: DeletePointsRequest = serde_json::from_str(json).unwrap();

        // Assert
        assert_eq!(request.collection, "docs");
        assert_eq!(request.ids, vec![1, 2]);
    }

    #[test]
    fn test_batch_search_request_deserialize() {
        // Arrange
        let json = r#"{"collection": "docs", "searches": [{"vector": [0.1, 0.2]}, {"vector": [0.3, 0.4], "topK": 5}]}"#;

        // Act
        let request: BatchSearchRequest = serde_json::from_str(json).unwrap();

        // Assert
        assert_eq!(request.collection, "docs");
        assert_eq!(request.searches.len(), 2);
        assert_eq!(request.searches[0].vector, vec![0.1, 0.2]);
        assert_eq!(request.searches[0].top_k, 10); // default
        assert_eq!(request.searches[1].vector, vec![0.3, 0.4]);
        assert_eq!(request.searches[1].top_k, 5);
    }

    #[test]
    fn test_point_output_serialize() {
        // Arrange
        let point = PointOutput {
            id: 1,
            vector: vec![0.1, 0.2, 0.3],
            payload: Some(serde_json::json!({"key": "value"})),
        };

        // Act
        let json = serde_json::to_string(&point).unwrap();

        // Assert
        assert!(json.contains("\"id\":1"));
        assert!(json.contains("\"vector\":[0.1,0.2,0.3]"));
        assert!(json.contains("\"key\":\"value\""));
    }

    #[test]
    fn test_text_search_request_deserialize() {
        // Arrange
        let json = r#"{"collection": "docs", "query": "machine learning"}"#;

        // Act
        let request: TextSearchRequest = serde_json::from_str(json).unwrap();

        // Assert
        assert_eq!(request.collection, "docs");
        assert_eq!(request.query, "machine learning");
        assert_eq!(request.top_k, 10); // default
    }

    #[test]
    fn test_hybrid_search_request_deserialize() {
        // Arrange
        let json = r#"{"collection": "docs", "vector": [0.1, 0.2], "query": "test"}"#;

        // Act
        let request: HybridSearchRequest = serde_json::from_str(json).unwrap();

        // Assert
        assert_eq!(request.collection, "docs");
        assert_eq!(request.vector, vec![0.1, 0.2]);
        assert_eq!(request.query, "test");
        assert_eq!(request.top_k, 10); // default
        assert!((request.vector_weight - 0.5).abs() < f32::EPSILON); // default
    }

    #[test]
    fn test_parse_storage_mode_full() {
        // Arrange & Act
        let result = parse_storage_mode("full");

        // Assert
        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_storage_mode_sq8() {
        // Arrange & Act
        let result = parse_storage_mode("sq8");

        // Assert
        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_storage_mode_binary() {
        // Arrange & Act
        let result = parse_storage_mode("binary");

        // Assert
        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_storage_mode_invalid() {
        // Arrange & Act
        let result = parse_storage_mode("unknown");

        // Assert
        assert!(result.is_err());
    }

    #[test]
    fn test_storage_mode_to_string() {
        use velesdb_core::StorageMode;

        // Arrange & Act & Assert
        assert_eq!(storage_mode_to_string(StorageMode::Full), "full");
        assert_eq!(storage_mode_to_string(StorageMode::SQ8), "sq8");
        assert_eq!(storage_mode_to_string(StorageMode::Binary), "binary");
    }

    #[test]
    fn test_search_response_serialize() {
        // Arrange
        let response = SearchResponse {
            results: vec![SearchResult {
                id: 1,
                score: 0.9,
                payload: None,
            }],
            timing_ms: 1.5,
        };

        // Act
        let json = serde_json::to_string(&response).unwrap();

        // Assert
        assert!(json.contains("\"results\""));
        assert!(json.contains("\"timingMs\":1.5"));
    }
}
