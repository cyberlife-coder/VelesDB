//! Tests for Tauri commands (TDD - written BEFORE implementation verification)

use crate::helpers::{metric_to_string, parse_metric, parse_storage_mode, storage_mode_to_string};
use crate::types::{
    default_metric, default_top_k, default_vector_weight, BatchSearchRequest, CollectionInfo,
    CreateCollectionRequest, DeletePointsRequest, GetPointsRequest, HybridSearchRequest,
    PointOutput, SearchRequest, SearchResponse, SearchResult, TextSearchRequest,
};

#[test]
fn test_parse_metric_cosine() {
    let result = parse_metric("cosine");
    assert!(result.is_ok());
}

#[test]
fn test_parse_metric_euclidean() {
    let result = parse_metric("euclidean");
    assert!(result.is_ok());
}

#[test]
fn test_parse_metric_l2_alias() {
    let result = parse_metric("l2");
    assert!(result.is_ok());
}

#[test]
fn test_parse_metric_dot() {
    let result = parse_metric("dot");
    assert!(result.is_ok());
}

#[test]
fn test_parse_metric_hamming() {
    let result = parse_metric("hamming");
    assert!(result.is_ok());
}

#[test]
fn test_parse_metric_jaccard() {
    let result = parse_metric("jaccard");
    assert!(result.is_ok());
}

#[test]
fn test_parse_metric_invalid() {
    let result = parse_metric("unknown");
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("Unknown metric"));
}

#[test]
fn test_parse_metric_case_insensitive() {
    assert!(parse_metric("COSINE").is_ok());
    assert!(parse_metric("Euclidean").is_ok());
    assert!(parse_metric("DOT").is_ok());
}

#[test]
fn test_metric_to_string() {
    use velesdb_core::distance::DistanceMetric;

    assert_eq!(metric_to_string(DistanceMetric::Cosine), "cosine");
    assert_eq!(metric_to_string(DistanceMetric::Euclidean), "euclidean");
    assert_eq!(metric_to_string(DistanceMetric::DotProduct), "dot");
    assert_eq!(metric_to_string(DistanceMetric::Hamming), "hamming");
    assert_eq!(metric_to_string(DistanceMetric::Jaccard), "jaccard");
}

#[test]
fn test_default_metric() {
    let metric = default_metric();
    assert_eq!(metric, "cosine");
}

#[test]
fn test_default_top_k() {
    let k = default_top_k();
    assert_eq!(k, 10);
}

#[test]
fn test_default_vector_weight() {
    let weight = default_vector_weight();
    assert!((weight - 0.5).abs() < f32::EPSILON);
}

#[test]
fn test_create_collection_request_deserialize() {
    let json = r#"{"name": "test", "dimension": 768}"#;
    let request: CreateCollectionRequest = serde_json::from_str(json).unwrap();

    assert_eq!(request.name, "test");
    assert_eq!(request.dimension, 768);
    assert_eq!(request.metric, "cosine");
    assert_eq!(request.storage_mode, "full");
}

#[test]
fn test_search_request_deserialize() {
    let json = r#"{"collection": "docs", "vector": [0.1, 0.2, 0.3]}"#;
    let request: SearchRequest = serde_json::from_str(json).unwrap();

    assert_eq!(request.collection, "docs");
    assert_eq!(request.vector, vec![0.1, 0.2, 0.3]);
    assert_eq!(request.top_k, 10);
}

#[test]
fn test_collection_info_serialize() {
    let info = CollectionInfo {
        name: "test".to_string(),
        dimension: 768,
        metric: "cosine".to_string(),
        count: 100,
        storage_mode: "full".to_string(),
    };
    let json = serde_json::to_string(&info).unwrap();

    assert!(json.contains("\"name\":\"test\""));
    assert!(json.contains("\"dimension\":768"));
    assert!(json.contains("\"metric\":\"cosine\""));
    assert!(json.contains("\"count\":100"));
    assert!(json.contains("\"storageMode\":\"full\""));
}

#[test]
fn test_search_result_serialize() {
    let result = SearchResult {
        id: 42,
        score: 0.95,
        payload: Some(serde_json::json!({"title": "Test"})),
    };
    let json = serde_json::to_string(&result).unwrap();

    assert!(json.contains("\"id\":42"));
    assert!(json.contains("\"score\":0.95"));
    assert!(json.contains("\"title\":\"Test\""));
}

#[test]
fn test_get_points_request_deserialize() {
    let json = r#"{"collection": "docs", "ids": [1, 2, 3]}"#;
    let request: GetPointsRequest = serde_json::from_str(json).unwrap();

    assert_eq!(request.collection, "docs");
    assert_eq!(request.ids, vec![1, 2, 3]);
}

#[test]
fn test_delete_points_request_deserialize() {
    let json = r#"{"collection": "docs", "ids": [1, 2]}"#;
    let request: DeletePointsRequest = serde_json::from_str(json).unwrap();

    assert_eq!(request.collection, "docs");
    assert_eq!(request.ids, vec![1, 2]);
}

#[test]
fn test_batch_search_request_deserialize() {
    let json = r#"{"collection": "docs", "searches": [{"vector": [0.1, 0.2]}, {"vector": [0.3, 0.4], "topK": 5}]}"#;
    let request: BatchSearchRequest = serde_json::from_str(json).unwrap();

    assert_eq!(request.collection, "docs");
    assert_eq!(request.searches.len(), 2);
    assert_eq!(request.searches[0].vector, vec![0.1, 0.2]);
    assert_eq!(request.searches[0].top_k, 10);
    assert_eq!(request.searches[1].vector, vec![0.3, 0.4]);
    assert_eq!(request.searches[1].top_k, 5);
}

#[test]
fn test_point_output_serialize() {
    let point = PointOutput {
        id: 1,
        vector: vec![0.1, 0.2, 0.3],
        payload: Some(serde_json::json!({"key": "value"})),
    };
    let json = serde_json::to_string(&point).unwrap();

    assert!(json.contains("\"id\":1"));
    assert!(json.contains("\"vector\":[0.1,0.2,0.3]"));
    assert!(json.contains("\"key\":\"value\""));
}

#[test]
fn test_text_search_request_deserialize() {
    let json = r#"{"collection": "docs", "query": "machine learning"}"#;
    let request: TextSearchRequest = serde_json::from_str(json).unwrap();

    assert_eq!(request.collection, "docs");
    assert_eq!(request.query, "machine learning");
    assert_eq!(request.top_k, 10);
}

#[test]
fn test_hybrid_search_request_deserialize() {
    let json = r#"{"collection": "docs", "vector": [0.1, 0.2], "query": "test"}"#;
    let request: HybridSearchRequest = serde_json::from_str(json).unwrap();

    assert_eq!(request.collection, "docs");
    assert_eq!(request.vector, vec![0.1, 0.2]);
    assert_eq!(request.query, "test");
    assert_eq!(request.top_k, 10);
    assert!((request.vector_weight - 0.5).abs() < f32::EPSILON);
}

#[test]
fn test_parse_storage_mode_full() {
    let result = parse_storage_mode("full");
    assert!(result.is_ok());
}

#[test]
fn test_parse_storage_mode_sq8() {
    let result = parse_storage_mode("sq8");
    assert!(result.is_ok());
}

#[test]
fn test_parse_storage_mode_binary() {
    let result = parse_storage_mode("binary");
    assert!(result.is_ok());
}

#[test]
fn test_parse_storage_mode_invalid() {
    let result = parse_storage_mode("unknown");
    assert!(result.is_err());
}

#[test]
fn test_storage_mode_to_string() {
    use velesdb_core::StorageMode;

    assert_eq!(storage_mode_to_string(StorageMode::Full), "full");
    assert_eq!(storage_mode_to_string(StorageMode::SQ8), "sq8");
    assert_eq!(storage_mode_to_string(StorageMode::Binary), "binary");
}

#[test]
fn test_search_response_serialize() {
    let response = SearchResponse {
        results: vec![SearchResult {
            id: 1,
            score: 0.9,
            payload: None,
        }],
        timing_ms: 1.5,
    };
    let json = serde_json::to_string(&response).unwrap();

    assert!(json.contains("\"results\""));
    assert!(json.contains("\"timingMs\":1.5"));
}
