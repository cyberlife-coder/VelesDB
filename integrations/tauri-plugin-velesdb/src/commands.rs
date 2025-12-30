//! Tauri commands for `VelesDB` operations.
//!
//! These commands are exposed to the frontend via the Tauri IPC system.

#![allow(clippy::missing_errors_doc)] // Tauri commands have implicit error handling

use serde::{Deserialize, Serialize};
use tauri::{command, AppHandle, Runtime, State};

use crate::error::{CommandError, Error, Result};
use crate::state::VelesDbState;

// ============================================================================
// Request/Response DTOs
// ============================================================================

/// Request to create a new collection.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateCollectionRequest {
    /// Collection name.
    pub name: String,
    /// Vector dimension.
    pub dimension: usize,
    /// Distance metric: "cosine", "euclidean", "dot", "hamming", "jaccard".
    #[serde(default = "default_metric")]
    pub metric: String,
    /// Storage mode: "full", "sq8", "binary".
    #[serde(default = "default_storage_mode")]
    pub storage_mode: String,
}

fn default_metric() -> String {
    "cosine".to_string()
}

fn default_storage_mode() -> String {
    "full".to_string()
}

/// Response for collection info.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectionInfo {
    /// Collection name.
    pub name: String,
    /// Vector dimension.
    pub dimension: usize,
    /// Distance metric.
    pub metric: String,
    /// Number of points.
    pub count: usize,
    /// Storage mode.
    pub storage_mode: String,
}

/// A point to insert/update.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PointInput {
    /// Point ID.
    pub id: u64,
    /// Vector data.
    pub vector: Vec<f32>,
    /// Optional payload (JSON object).
    pub payload: Option<serde_json::Value>,
}

/// Request to upsert points.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpsertRequest {
    /// Collection name.
    pub collection: String,
    /// Points to upsert.
    pub points: Vec<PointInput>,
}

/// Request to get points by IDs.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetPointsRequest {
    /// Collection name.
    pub collection: String,
    /// Point IDs to retrieve.
    pub ids: Vec<u64>,
}

/// Request to delete points by IDs.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeletePointsRequest {
    /// Collection name.
    pub collection: String,
    /// Point IDs to delete.
    pub ids: Vec<u64>,
}

/// Request to search vectors.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchRequest {
    /// Collection name.
    pub collection: String,
    /// Query vector.
    pub vector: Vec<f32>,
    /// Number of results.
    #[serde(default = "default_top_k")]
    pub top_k: usize,
}

/// Request for batch search.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchSearchRequest {
    /// Collection name.
    pub collection: String,
    /// Query vectors.
    pub vectors: Vec<Vec<f32>>,
    /// Number of results per query.
    #[serde(default = "default_top_k")]
    pub top_k: usize,
}

fn default_top_k() -> usize {
    10
}

/// Request for text search.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextSearchRequest {
    /// Collection name.
    pub collection: String,
    /// Text query.
    pub query: String,
    /// Number of results.
    #[serde(default = "default_top_k")]
    pub top_k: usize,
}

/// Request for hybrid search.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HybridSearchRequest {
    /// Collection name.
    pub collection: String,
    /// Query vector.
    pub vector: Vec<f32>,
    /// Text query.
    pub query: String,
    /// Number of results.
    #[serde(default = "default_top_k")]
    pub top_k: usize,
    /// Weight for vector results (0.0-1.0).
    #[serde(default = "default_vector_weight")]
    pub vector_weight: f32,
}

fn default_vector_weight() -> f32 {
    0.5
}

/// Request for `VelesQL` query.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryRequest {
    /// `VelesQL` query string.
    pub query: String,
    /// Query parameters.
    #[serde(default)]
    pub params: std::collections::HashMap<String, serde_json::Value>,
}

/// Search result.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    /// Point ID.
    pub id: u64,
    /// Similarity/distance score.
    pub score: f32,
    /// Point payload.
    pub payload: Option<serde_json::Value>,
}

/// Point output for get operations.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PointOutput {
    /// Point ID.
    pub id: u64,
    /// Vector data.
    pub vector: Vec<f32>,
    /// Point payload.
    pub payload: Option<serde_json::Value>,
}

/// Response for search operations.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResponse {
    /// Search results.
    pub results: Vec<SearchResult>,
    /// Query time in milliseconds.
    pub timing_ms: f64,
}

// ============================================================================
// Helper functions
// ============================================================================

fn parse_metric(metric: &str) -> Result<velesdb_core::distance::DistanceMetric> {
    use velesdb_core::distance::DistanceMetric;
    match metric.to_lowercase().as_str() {
        "cosine" => Ok(DistanceMetric::Cosine),
        "euclidean" | "l2" => Ok(DistanceMetric::Euclidean),
        "dot" | "dotproduct" | "inner" => Ok(DistanceMetric::DotProduct),
        "hamming" => Ok(DistanceMetric::Hamming),
        "jaccard" => Ok(DistanceMetric::Jaccard),
        _ => Err(Error::InvalidConfig(format!(
            "Unknown metric '{metric}'. Use: cosine, euclidean, dot, hamming, jaccard"
        ))),
    }
}

fn metric_to_string(metric: velesdb_core::distance::DistanceMetric) -> String {
    use velesdb_core::distance::DistanceMetric;
    match metric {
        DistanceMetric::Cosine => "cosine",
        DistanceMetric::Euclidean => "euclidean",
        DistanceMetric::DotProduct => "dot",
        DistanceMetric::Hamming => "hamming",
        DistanceMetric::Jaccard => "jaccard",
    }
    .to_string()
}

fn parse_storage_mode(mode: &str) -> Result<velesdb_core::StorageMode> {
    use velesdb_core::StorageMode;
    match mode.to_lowercase().as_str() {
        "full" | "f32" => Ok(StorageMode::Full),
        "sq8" | "int8" => Ok(StorageMode::SQ8),
        "binary" | "bit" => Ok(StorageMode::Binary),
        _ => Err(Error::InvalidConfig(format!(
            "Invalid storage_mode '{mode}'. Use 'full', 'sq8', or 'binary'"
        ))),
    }
}

fn storage_mode_to_string(mode: velesdb_core::StorageMode) -> String {
    use velesdb_core::StorageMode;
    match mode {
        StorageMode::Full => "full",
        StorageMode::SQ8 => "sq8",
        StorageMode::Binary => "binary",
    }
    .to_string()
}

// ============================================================================
// Tauri Commands
// ============================================================================

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

    let results = state
        .with_db(|db| {
            let coll = db
                .get_collection(&request.collection)
                .ok_or_else(|| Error::CollectionNotFound(request.collection.clone()))?;

            let search_results = coll.search(&request.vector, request.top_k)?;
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

            let query_refs: Vec<&[f32]> = request.vectors.iter().map(Vec::as_slice).collect();
            let results = coll.search_batch_parallel(&query_refs, request.top_k)?;

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

    let results = state
        .with_db(|db| {
            let coll = db
                .get_collection(&request.collection)
                .ok_or_else(|| Error::CollectionNotFound(request.collection.clone()))?;

            let search_results = coll.text_search(&request.query, request.top_k);
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

    let results = state
        .with_db(|db| {
            let coll = db
                .get_collection(&request.collection)
                .ok_or_else(|| Error::CollectionNotFound(request.collection.clone()))?;

            let search_results = coll.hybrid_search(
                &request.vector,
                &request.query,
                request.top_k,
                Some(request.vector_weight),
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

            // For now, just return empty results for non-search queries
            // Full VelesQL execution would require more complex logic
            #[allow(clippy::cast_possible_truncation)]
            let limit = parsed.select.limit.unwrap_or(10) as usize;

            // Simple text search if MATCH is present in query
            if request.query.to_lowercase().contains("match") {
                // Extract text between quotes after MATCH
                if let Some(start_idx) = request.query.find('\'') {
                    if let Some(end_idx) = request.query[start_idx + 1..].find('\'') {
                        let search_text = &request.query[start_idx + 1..start_idx + 1 + end_idx];
                        let search_results = coll.text_search(search_text, limit);
                        return Ok(search_results
                            .into_iter()
                            .map(|r| SearchResult {
                                id: r.point.id,
                                score: r.score,
                                payload: r.point.payload,
                            })
                            .collect::<Vec<_>>());
                    }
                }
            }

            Ok(Vec::new())
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
}
