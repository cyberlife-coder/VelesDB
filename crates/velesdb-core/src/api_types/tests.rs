//! Tests for the `api_types` module: serialization, defaults, edge cases, and utilities.

use serde_json::json;

use super::*;

// ============================================================================
// A. Serialization round-trip / deserialization tests (~15)
// ============================================================================

#[test]
fn deserialize_create_collection_request_full() {
    let input = json!({
        "name": "documents",
        "dimension": 768,
        "metric": "euclidean",
        "storage_mode": "sq8",
        "collection_type": "vector",
        "hnsw_m": 32,
        "hnsw_ef_construction": 400
    });
    let req: CreateCollectionRequest = serde_json::from_value(input).unwrap();
    assert_eq!(req.name, "documents");
    assert_eq!(req.dimension, Some(768));
    assert_eq!(req.metric, "euclidean");
    assert_eq!(req.storage_mode, "sq8");
    assert_eq!(req.collection_type, "vector");
    assert_eq!(req.hnsw_m, Some(32));
    assert_eq!(req.hnsw_ef_construction, Some(400));
}

#[test]
fn deserialize_create_collection_request_defaults() {
    let input = json!({ "name": "minimal" });
    let req: CreateCollectionRequest = serde_json::from_value(input).unwrap();
    assert_eq!(req.name, "minimal");
    assert_eq!(req.dimension, None);
    assert_eq!(req.metric, "cosine");
    assert_eq!(req.storage_mode, "full");
    assert_eq!(req.collection_type, "vector");
    assert_eq!(req.hnsw_m, None);
    assert_eq!(req.hnsw_ef_construction, None);
}

#[test]
fn deserialize_search_request_full() {
    let input = json!({
        "vector": [0.1, 0.2, 0.3],
        "top_k": 20,
        "mode": "accurate",
        "ef_search": 256,
        "timeout_ms": 5000,
        "filter": {"category": "tech"},
        "sparse_index": "title_sparse"
    });
    let req: SearchRequest = serde_json::from_value(input).unwrap();
    assert_eq!(req.vector, vec![0.1, 0.2, 0.3]);
    assert_eq!(req.top_k, 20);
    assert_eq!(req.mode.as_deref(), Some("accurate"));
    assert_eq!(req.ef_search, Some(256));
    assert_eq!(req.timeout_ms, Some(5000));
    assert!(req.filter.is_some());
    assert_eq!(req.sparse_index.as_deref(), Some("title_sparse"));
}

#[test]
fn deserialize_search_request_defaults() {
    let input = json!({});
    let req: SearchRequest = serde_json::from_value(input).unwrap();
    assert!(req.vector.is_empty());
    assert_eq!(req.top_k, 10);
    assert_eq!(req.mode, None);
    assert_eq!(req.ef_search, None);
    assert_eq!(req.timeout_ms, None);
    assert_eq!(req.filter, None);
    assert!(req.sparse_vector.is_none());
    assert!(req.sparse_vectors.is_none());
    assert_eq!(req.sparse_index, None);
    assert!(req.fusion.is_none());
}

#[test]
fn deserialize_batch_search_request() {
    let input = json!({
        "searches": [
            {"vector": [1.0, 2.0], "top_k": 5},
            {"vector": [3.0, 4.0]}
        ]
    });
    let req: BatchSearchRequest = serde_json::from_value(input).unwrap();
    assert_eq!(req.searches.len(), 2);
    assert_eq!(req.searches[0].top_k, 5);
    assert_eq!(req.searches[1].top_k, 10); // default
}

#[test]
fn deserialize_text_search_request() {
    let input = json!({
        "query": "rust programming",
        "top_k": 15,
        "filter": {"lang": "en"}
    });
    let req: TextSearchRequest = serde_json::from_value(input).unwrap();
    assert_eq!(req.query, "rust programming");
    assert_eq!(req.top_k, 15);
    assert!(req.filter.is_some());
}

#[test]
fn deserialize_hybrid_search_request() {
    let input = json!({
        "vector": [0.5, 0.6],
        "query": "vector database",
        "top_k": 25,
        "vector_weight": 0.7
    });
    let req: HybridSearchRequest = serde_json::from_value(input).unwrap();
    assert_eq!(req.vector, vec![0.5, 0.6]);
    assert_eq!(req.query, "vector database");
    assert_eq!(req.top_k, 25);
    assert!((req.vector_weight - 0.7).abs() < f32::EPSILON);
}

#[test]
fn deserialize_hybrid_search_request_defaults() {
    let input = json!({
        "vector": [1.0],
        "query": "test"
    });
    let req: HybridSearchRequest = serde_json::from_value(input).unwrap();
    assert_eq!(req.top_k, 10);
    assert!((req.vector_weight - 0.5).abs() < f32::EPSILON);
    assert_eq!(req.filter, None);
}

#[test]
fn deserialize_multi_query_search_request() {
    let input = json!({
        "vectors": [[1.0, 2.0], [3.0, 4.0]],
        "top_k": 5,
        "strategy": "weighted",
        "rrf_k": 30,
        "avg_weight": 0.4,
        "max_weight": 0.4,
        "hit_weight": 0.2,
        "filter": null
    });
    let req: MultiQuerySearchRequest = serde_json::from_value(input).unwrap();
    assert_eq!(req.vectors.len(), 2);
    assert_eq!(req.top_k, 5);
    assert_eq!(req.strategy, "weighted");
    assert_eq!(req.rrf_k, 30);
    assert!((req.avg_weight - 0.4).abs() < f32::EPSILON);
    assert!((req.max_weight - 0.4).abs() < f32::EPSILON);
    assert!((req.hit_weight - 0.2).abs() < f32::EPSILON);
}

#[test]
fn deserialize_multi_query_search_request_defaults() {
    let input = json!({ "vectors": [[1.0]] });
    let req: MultiQuerySearchRequest = serde_json::from_value(input).unwrap();
    assert_eq!(req.top_k, 10);
    assert_eq!(req.strategy, "rrf");
    assert_eq!(req.rrf_k, 60);
    assert!((req.avg_weight - 0.5).abs() < f32::EPSILON);
    assert!((req.max_weight - 0.3).abs() < f32::EPSILON);
    assert!((req.hit_weight - 0.2).abs() < f32::EPSILON);
}

#[test]
fn deserialize_upsert_points_request() {
    let input = json!({
        "points": [{
            "id": 42,
            "vector": [0.1, 0.2],
            "payload": {"title": "hello"}
        }]
    });
    let req: UpsertPointsRequest = serde_json::from_value(input).unwrap();
    assert_eq!(req.points.len(), 1);
    assert_eq!(req.points[0].id, 42);
    assert_eq!(req.points[0].vector, vec![0.1, 0.2]);
    assert!(req.points[0].payload.is_some());
}

#[test]
fn deserialize_create_index_request() {
    let input = json!({
        "label": "Person",
        "property": "email",
        "index_type": "range"
    });
    let req: CreateIndexRequest = serde_json::from_value(input).unwrap();
    assert_eq!(req.label, "Person");
    assert_eq!(req.property, "email");
    assert_eq!(req.index_type, "range");
}

#[test]
fn deserialize_create_index_request_default_type() {
    let input = json!({ "label": "Doc", "property": "tag" });
    let req: CreateIndexRequest = serde_json::from_value(input).unwrap();
    assert_eq!(req.index_type, "hash");
}

#[test]
fn round_trip_collection_config_response() {
    let resp = CollectionConfigResponse {
        name: "docs".to_string(),
        dimension: 128,
        metric: "cosine".to_string(),
        storage_mode: "full".to_string(),
        point_count: 1000,
        metadata_only: false,
        graph_schema: Some(json!({"labels": ["Person"]})),
        embedding_dimension: Some(128),
    };
    let serialized = serde_json::to_value(&resp).unwrap();
    let deserialized: CollectionConfigResponse = serde_json::from_value(serialized).unwrap();
    assert_eq!(deserialized.name, "docs");
    assert_eq!(deserialized.dimension, 128);
    assert_eq!(deserialized.metric, "cosine");
    assert_eq!(deserialized.storage_mode, "full");
    assert_eq!(deserialized.point_count, 1000);
    assert!(!deserialized.metadata_only);
    assert!(deserialized.graph_schema.is_some());
    assert_eq!(deserialized.embedding_dimension, Some(128));
}

#[test]
fn round_trip_search_ids_response() {
    let resp = SearchIdsResponse {
        results: vec![
            IdScoreResult { id: 1, score: 0.95 },
            IdScoreResult { id: 2, score: 0.80 },
        ],
    };
    let serialized = serde_json::to_value(&resp).unwrap();
    let deserialized: SearchIdsResponse = serde_json::from_value(serialized).unwrap();
    assert_eq!(deserialized.results.len(), 2);
    assert_eq!(deserialized.results[0].id, 1);
    assert!((deserialized.results[0].score - 0.95).abs() < f32::EPSILON);
}

#[test]
fn round_trip_query_type_enum() {
    // All variants serialize to lowercase strings.
    let cases = [
        (QueryType::Search, "\"search\""),
        (QueryType::Aggregation, "\"aggregation\""),
        (QueryType::Rows, "\"rows\""),
        (QueryType::Graph, "\"graph\""),
    ];
    for (variant, expected_json) in &cases {
        let serialized = serde_json::to_string(variant).unwrap();
        assert_eq!(&serialized, expected_json);
        let deserialized: QueryType = serde_json::from_str(&serialized).unwrap();
        assert_eq!(&deserialized, variant);
    }
}

// ============================================================================
// B. Default value function tests (~8)
// ============================================================================

#[test]
fn test_default_metric() {
    assert_eq!(default_metric(), "cosine");
}

#[test]
fn test_default_storage_mode() {
    assert_eq!(default_storage_mode(), "full");
}

#[test]
fn test_default_top_k() {
    assert_eq!(default_top_k(), 10);
}

#[test]
fn test_default_vector_weight() {
    assert!((default_vector_weight() - 0.5).abs() < f32::EPSILON);
}

#[test]
fn test_default_collection_type() {
    assert_eq!(default_collection_type(), "vector");
}

#[test]
fn test_default_fusion_strategy() {
    assert_eq!(default_fusion_strategy(), "rrf");
}

#[test]
fn test_default_rrf_k() {
    assert_eq!(default_rrf_k(), 60);
}

#[test]
fn test_mode_to_ef_search_all_modes() {
    assert_eq!(mode_to_ef_search("fast"), Some(64));
    assert_eq!(mode_to_ef_search("balanced"), Some(128));
    assert_eq!(mode_to_ef_search("accurate"), Some(512));
    assert_eq!(mode_to_ef_search("perfect"), Some(usize::MAX));
    assert_eq!(mode_to_ef_search("unknown_mode"), None);
    assert_eq!(mode_to_ef_search(""), None);
}

// ============================================================================
// C. Edge case tests (~10)
// ============================================================================

#[test]
fn search_request_empty_vector() {
    let input = json!({ "vector": [] });
    let req: SearchRequest = serde_json::from_value(input).unwrap();
    assert!(req.vector.is_empty());
    assert_eq!(req.top_k, 10);
}

#[test]
fn upsert_request_null_payload() {
    let input = json!({
        "points": [{
            "id": 1,
            "vector": [0.5],
            "payload": null
        }]
    });
    let req: UpsertPointsRequest = serde_json::from_value(input).unwrap();
    assert_eq!(req.points[0].payload, None);
}

#[test]
fn missing_optional_fields_deserialize_to_none() {
    let input = json!({
        "points": [{
            "id": 99,
            "vector": [1.0, 2.0]
        }]
    });
    let req: UpsertPointsRequest = serde_json::from_value(input).unwrap();
    assert_eq!(req.points[0].payload, None);
    assert!(req.points[0].sparse_vector.is_none());
    assert!(req.points[0].sparse_vectors.is_none());
}

#[test]
fn serialize_search_response_empty_results() {
    let resp = SearchResponse { results: vec![] };
    let json = serde_json::to_value(&resp).unwrap();
    assert_eq!(json["results"], json!([]));
}

#[test]
fn create_collection_zero_dimension() {
    let input = json!({
        "name": "zero_dim",
        "dimension": 0
    });
    let req: CreateCollectionRequest = serde_json::from_value(input).unwrap();
    assert_eq!(req.dimension, Some(0));
}

#[test]
fn search_request_large_top_k() {
    let input = json!({
        "vector": [1.0],
        "top_k": 1_000_000
    });
    let req: SearchRequest = serde_json::from_value(input).unwrap();
    assert_eq!(req.top_k, 1_000_000);
}

#[test]
fn create_collection_special_characters_in_name() {
    let input = json!({
        "name": "my-collection_v2.1 (test)",
        "dimension": 3
    });
    let req: CreateCollectionRequest = serde_json::from_value(input).unwrap();
    assert_eq!(req.name, "my-collection_v2.1 (test)");
}

#[test]
fn sparse_vector_parallel_format() {
    let input = json!({
        "indices": [42, 1337],
        "values": [0.5, 1.2]
    });
    let sparse: SparseVectorInput = serde_json::from_value(input).unwrap();
    match sparse {
        SparseVectorInput::Parallel { indices, values } => {
            assert_eq!(indices, vec![42, 1337]);
            assert_eq!(values, vec![0.5, 1.2]);
        }
        SparseVectorInput::Dict(_) => panic!("Expected Parallel variant"),
    }
}

#[test]
fn sparse_vector_dict_format() {
    let input = json!({
        "42": 0.5,
        "1337": 1.2
    });
    let sparse: SparseVectorInput = serde_json::from_value(input).unwrap();
    match sparse {
        SparseVectorInput::Dict(map) => {
            assert_eq!(map.len(), 2);
            assert!((map["42"] - 0.5).abs() < f32::EPSILON);
            assert!((map["1337"] - 1.2).abs() < f32::EPSILON);
        }
        SparseVectorInput::Parallel { .. } => panic!("Expected Dict variant"),
    }
}

#[test]
fn empty_batch_search_request() {
    let input = json!({ "searches": [] });
    let req: BatchSearchRequest = serde_json::from_value(input).unwrap();
    assert!(req.searches.is_empty());
}

#[test]
fn filter_with_nested_conditions() {
    let input = json!({
        "vector": [1.0],
        "filter": {
            "$and": [
                {"category": {"$eq": "tech"}},
                {"$or": [
                    {"year": {"$gte": 2020}},
                    {"featured": true}
                ]}
            ]
        }
    });
    let req: SearchRequest = serde_json::from_value(input).unwrap();
    let filter = req.filter.unwrap();
    assert!(filter["$and"].is_array());
    assert_eq!(filter["$and"].as_array().unwrap().len(), 2);
}

// ============================================================================
// D. Utility function / constant tests (~3+)
// ============================================================================

#[test]
fn test_mode_to_ef_search_case_insensitive() {
    assert_eq!(mode_to_ef_search("FAST"), Some(64));
    assert_eq!(mode_to_ef_search("Balanced"), Some(128));
    assert_eq!(mode_to_ef_search("ACCURATE"), Some(512));
    assert_eq!(mode_to_ef_search("Perfect"), Some(usize::MAX));
}

#[test]
fn test_default_index_type() {
    assert_eq!(default_index_type(), "hash");
}

#[test]
fn test_velesql_contract_version() {
    assert_eq!(VELESQL_CONTRACT_VERSION, "3.0.0");
}

#[test]
fn test_default_fusion_weights() {
    assert!((default_avg_weight() - 0.5).abs() < f32::EPSILON);
    assert!((default_max_weight() - 0.3).abs() < f32::EPSILON);
    assert!((default_hit_weight() - 0.2).abs() < f32::EPSILON);
}

// ============================================================================
// E. mode_to_search_quality advanced modes (Custom, Adaptive)
// ============================================================================

#[cfg(feature = "persistence")]
#[test]
fn test_mode_to_search_quality_named_modes() {
    use super::mode_to_search_quality;
    assert!(matches!(
        mode_to_search_quality("fast"),
        Some(crate::SearchQuality::Fast)
    ));
    assert!(matches!(
        mode_to_search_quality("balanced"),
        Some(crate::SearchQuality::Balanced)
    ));
    assert!(matches!(
        mode_to_search_quality("accurate"),
        Some(crate::SearchQuality::Accurate)
    ));
    assert!(matches!(
        mode_to_search_quality("perfect"),
        Some(crate::SearchQuality::Perfect)
    ));
    assert!(matches!(
        mode_to_search_quality("autotune"),
        Some(crate::SearchQuality::AutoTune)
    ));
    assert!(matches!(
        mode_to_search_quality("auto"),
        Some(crate::SearchQuality::AutoTune)
    ));
}

#[cfg(feature = "persistence")]
#[test]
fn test_mode_to_search_quality_custom() {
    use super::mode_to_search_quality;
    let q = mode_to_search_quality("custom:256");
    assert!(matches!(q, Some(crate::SearchQuality::Custom(256))));
}

#[cfg(feature = "persistence")]
#[test]
fn test_mode_to_search_quality_adaptive() {
    use super::mode_to_search_quality;
    let q = mode_to_search_quality("adaptive:32:512");
    assert!(matches!(
        q,
        Some(crate::SearchQuality::Adaptive {
            min_ef: 32,
            max_ef: 512
        })
    ));
}

#[cfg(feature = "persistence")]
#[test]
fn test_mode_to_search_quality_invalid_custom() {
    use super::mode_to_search_quality;
    assert!(mode_to_search_quality("custom:abc").is_none());
    assert!(mode_to_search_quality("custom:").is_none());
}

#[cfg(feature = "persistence")]
#[test]
fn test_mode_to_search_quality_invalid_adaptive() {
    use super::mode_to_search_quality;
    assert!(mode_to_search_quality("adaptive:32").is_none());
    assert!(mode_to_search_quality("adaptive:a:b").is_none());
    // Inverted range (min > max) is rejected
    assert!(mode_to_search_quality("adaptive:512:32").is_none());
}

#[cfg(feature = "persistence")]
#[test]
fn test_mode_to_search_quality_unknown() {
    use super::mode_to_search_quality;
    assert!(mode_to_search_quality("nonexistent").is_none());
    assert!(mode_to_search_quality("").is_none());
}

// ============================================================================
// Additional edge-case tests for response serialization
// ============================================================================

#[test]
fn serialize_error_response() {
    let resp = ErrorResponse {
        error: "collection not found".to_string(),
        code: None,
    };
    let json = serde_json::to_value(&resp).unwrap();
    assert_eq!(json["error"], "collection not found");
    // code: None is skipped by serde
    assert!(!json.as_object().unwrap().contains_key("code"));
}

#[test]
fn serialize_error_response_with_code() {
    let resp = ErrorResponse {
        error: "Dimension mismatch".to_string(),
        code: Some("VELES-004".to_string()),
    };
    let json = serde_json::to_value(&resp).unwrap();
    assert_eq!(json["error"], "Dimension mismatch");
    assert_eq!(json["code"], "VELES-004");
}

#[test]
fn serialize_collection_config_skips_none_graph_schema() {
    let resp = CollectionConfigResponse {
        name: "simple".to_string(),
        dimension: 64,
        metric: "cosine".to_string(),
        storage_mode: "full".to_string(),
        point_count: 0,
        metadata_only: true,
        graph_schema: None,
        embedding_dimension: None,
    };
    let json = serde_json::to_value(&resp).unwrap();
    // `skip_serializing_if = "Option::is_none"` omits the key entirely
    assert!(!json.as_object().unwrap().contains_key("graph_schema"));
    assert!(!json
        .as_object()
        .unwrap()
        .contains_key("embedding_dimension"));
}

#[test]
fn serialize_unified_query_response_skips_empty_warnings() {
    let resp = UnifiedQueryResponse {
        query_type: QueryType::Search,
        count: 0,
        timing_ms: 1.5,
        results: json!([]),
        warnings: vec![],
    };
    let json = serde_json::to_value(&resp).unwrap();
    // `skip_serializing_if = "Vec::is_empty"` omits warnings key
    assert!(!json.as_object().unwrap().contains_key("warnings"));
    // `type` field should be the serde-renamed key
    assert_eq!(json["type"], "search");
}

#[test]
fn deserialize_query_request_with_params() {
    let input = json!({
        "query": "SELECT * FROM docs WHERE vector NEAR $v LIMIT 10",
        "params": {"v": [0.1, 0.2, 0.3]},
        "collection": "docs"
    });
    let req: QueryRequest = serde_json::from_value(input).unwrap();
    assert!(req.query.contains("NEAR"));
    assert!(req.params.contains_key("v"));
    assert_eq!(req.collection.as_deref(), Some("docs"));
}

#[test]
fn deserialize_guardrails_config_request_partial() {
    let input = json!({
        "max_depth": 5,
        "timeout_ms": 10000
    });
    let req: GuardRailsConfigRequest = serde_json::from_value(input).unwrap();
    assert_eq!(req.max_depth, Some(5));
    assert_eq!(req.timeout_ms, Some(10000));
    assert_eq!(req.max_cardinality, None);
    assert_eq!(req.memory_limit_bytes, None);
    assert_eq!(req.rate_limit_qps, None);
    assert_eq!(req.circuit_failure_threshold, None);
    assert_eq!(req.circuit_recovery_seconds, None);
}

#[test]
fn deserialize_stream_insert_request() {
    let input = json!({
        "id": 7,
        "vector": [0.1, 0.2, 0.3],
        "payload": {"source": "api"}
    });
    let req: StreamInsertRequest = serde_json::from_value(input).unwrap();
    assert_eq!(req.id, 7);
    assert_eq!(req.vector, vec![0.1, 0.2, 0.3]);
    assert!(req.payload.is_some());
}

#[test]
fn deserialize_fusion_request() {
    let input = json!({
        "strategy": "rsf",
        "dense_w": 0.7,
        "sparse_w": 0.3
    });
    let req: FusionRequest = serde_json::from_value(input).unwrap();
    assert_eq!(req.strategy, "rsf");
    assert_eq!(req.k, None);
    assert!((req.dense_w.unwrap() - 0.7).abs() < f32::EPSILON);
    assert!((req.sparse_w.unwrap() - 0.3).abs() < f32::EPSILON);
}
