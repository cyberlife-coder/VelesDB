//! Tests for Tauri commands (TDD - written BEFORE implementation verification)

use crate::helpers::{metric_to_string, parse_metric, parse_storage_mode, storage_mode_to_string};
use crate::types::{
    default_metric, default_top_k, default_vector_weight, BatchSearchRequest, CollectionInfo,
    CreateCollectionRequest, DeletePointsRequest, GetPointsRequest, HybridResult,
    HybridSearchRequest, PointOutput, QueryRequest, QueryResponse, SearchRequest, SearchResponse,
    SearchResult, TextSearchRequest,
};
use toml;

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

// ============================================================================
// VelesQL Query Tests (EPIC-031 US-012)
// ============================================================================

#[test]
fn test_query_request_deserialize_simple() {
    let json = r#"{"query": "MATCH (d:Doc) RETURN d"}"#;
    let request: QueryRequest = serde_json::from_str(json).unwrap();

    assert_eq!(request.query, "MATCH (d:Doc) RETURN d");
    assert!(request.params.is_empty());
}

#[test]
fn test_query_request_deserialize_with_params() {
    let json = r#"{
        "query": "MATCH (d:Doc) WHERE similarity(d.embedding, $q) > 0.7 RETURN d",
        "params": {"q": [0.1, 0.2, 0.3]}
    }"#;
    let request: QueryRequest = serde_json::from_str(json).unwrap();

    assert!(request.query.contains("similarity"));
    assert!(request.params.contains_key("q"));
}

#[test]
fn test_query_request_camel_case() {
    // Ensure camelCase deserialization works
    let json = r#"{"query": "SELECT * FROM docs"}"#;
    let request: QueryRequest = serde_json::from_str(json).unwrap();
    assert_eq!(request.query, "SELECT * FROM docs");
}

#[test]
fn test_hybrid_result_serialize() {
    let result = HybridResult {
        node_id: 42,
        vector_score: Some(0.95),
        graph_score: Some(0.88),
        fused_score: 0.92,
        bindings: Some(serde_json::json!({"title": "Test Document"})),
        column_data: None,
    };
    let json = serde_json::to_string(&result).unwrap();

    assert!(json.contains("\"nodeId\":42"));
    assert!(json.contains("\"vectorScore\":0.95"));
    assert!(json.contains("\"graphScore\":0.88"));
    assert!(json.contains("\"fusedScore\":0.92"));
    assert!(json.contains("\"title\":\"Test Document\""));
}

#[test]
fn test_hybrid_result_serialize_minimal() {
    let result = HybridResult {
        node_id: 1,
        vector_score: None,
        graph_score: None,
        fused_score: 0.5,
        bindings: None,
        column_data: None,
    };
    let json = serde_json::to_string(&result).unwrap();

    assert!(json.contains("\"nodeId\":1"));
    assert!(json.contains("\"fusedScore\":0.5"));
    assert!(json.contains("\"vectorScore\":null"));
}

#[test]
fn test_hybrid_result_with_column_data() {
    let result = HybridResult {
        node_id: 10,
        vector_score: Some(0.9),
        graph_score: None,
        fused_score: 0.9,
        bindings: None,
        column_data: Some(serde_json::json!({"price": 99.99, "currency": "USD"})),
    };
    let json = serde_json::to_string(&result).unwrap();

    assert!(json.contains("\"columnData\""));
    assert!(json.contains("\"price\":99.99"));
    assert!(json.contains("\"currency\":\"USD\""));
}

#[test]
fn test_query_response_serialize() {
    let response = QueryResponse {
        results: vec![
            HybridResult {
                node_id: 1,
                vector_score: Some(0.95),
                graph_score: None,
                fused_score: 0.95,
                bindings: Some(serde_json::json!({"name": "Doc1"})),
                column_data: None,
            },
            HybridResult {
                node_id: 2,
                vector_score: Some(0.85),
                graph_score: None,
                fused_score: 0.85,
                bindings: Some(serde_json::json!({"name": "Doc2"})),
                column_data: None,
            },
        ],
        timing_ms: 2.5,
    };
    let json = serde_json::to_string(&response).unwrap();

    assert!(json.contains("\"results\""));
    assert!(json.contains("\"timingMs\":2.5"));
    assert!(json.contains("\"nodeId\":1"));
    assert!(json.contains("\"nodeId\":2"));
}

#[test]
fn test_query_response_empty_results() {
    let response = QueryResponse {
        results: vec![],
        timing_ms: 0.1,
    };
    let json = serde_json::to_string(&response).unwrap();

    assert!(json.contains("\"results\":[]"));
    assert!(json.contains("\"timingMs\":0.1"));
}

// ============================================================================
// Permission Regression Tests (Issue #169)
// ============================================================================

/// Canonical list of all commands registered in lib.rs invoke_handler.
///
/// IMPORTANT: This list MUST be kept in sync with:
/// - `src/lib.rs` invoke_handler registration
/// - `build.rs` COMMANDS array
/// - `permissions/default.toml` [default] permissions
///
/// When adding a new command:
/// 1. Add the command function to commands.rs or commands_graph.rs
/// 2. Register it in lib.rs invoke_handler
/// 3. Add it to build.rs COMMANDS array (triggers permission file generation)
/// 4. Add "allow-{command-name}" to default.toml [default] section
/// 5. Add it to this REGISTERED_COMMANDS array
const REGISTERED_COMMANDS: &[&str] = &[
    // Collection management
    "create_collection",
    "create_metadata_collection",
    "delete_collection",
    "list_collections",
    "get_collection",
    "is_empty",
    "flush",
    // Point operations
    "upsert",
    "upsert_metadata",
    "get_points",
    "delete_points",
    // Search operations
    "search",
    "batch_search",
    "text_search",
    "hybrid_search",
    "multi_query_search",
    "query",
    // AgentMemory (semantic)
    "semantic_store",
    "semantic_query",
    // Knowledge Graph
    "add_edge",
    "get_edges",
    "traverse_graph",
    "get_node_degree",
];

/// Test to ensure all registered commands have corresponding permissions
/// in the [default] section specifically (not just anywhere in the file).
///
/// Uses TOML parsing for robust section-specific validation.
///
/// Regression test for: <https://github.com/cyberlife-coder/VelesDB/issues/169>
#[test]
fn test_all_commands_have_default_permissions_toml_parsed() {
    let default_toml_content = include_str!("../permissions/default.toml");
    let parsed: toml::Value =
        toml::from_str(default_toml_content).expect("Failed to parse default.toml as valid TOML");

    let default_section = parsed
        .get("default")
        .expect("Missing [default] section in default.toml");

    let permissions = default_section
        .get("permissions")
        .expect("Missing 'permissions' array in [default] section")
        .as_array()
        .expect("'permissions' should be an array");

    let permission_strings: Vec<&str> = permissions.iter().filter_map(|v| v.as_str()).collect();

    for cmd in REGISTERED_COMMANDS {
        let expected_permission = format!("allow-{}", cmd.replace('_', "-"));
        assert!(
            permission_strings.contains(&expected_permission.as_str()),
            "Missing permission '{expected_permission}' in [default] section for command '{cmd}'.\n\
             Add '\"{expected_permission}\"' to the [default] permissions array in default.toml.\n\
             Note: The permission must be in the [default] section, not just anywhere in the file."
        );
    }
}

/// Test that the number of permissions in [default] matches the number of registered commands.
/// This catches cases where permissions exist but commands were removed.
#[test]
fn test_default_permissions_count_matches_commands() {
    let default_toml_content = include_str!("../permissions/default.toml");
    let parsed: toml::Value =
        toml::from_str(default_toml_content).expect("Failed to parse default.toml as valid TOML");

    let default_section = parsed
        .get("default")
        .expect("Missing [default] section in default.toml");

    let permissions = default_section
        .get("permissions")
        .expect("Missing 'permissions' array in [default] section")
        .as_array()
        .expect("'permissions' should be an array");

    assert_eq!(
        permissions.len(),
        REGISTERED_COMMANDS.len(),
        "Mismatch between number of permissions ({}) and registered commands ({}).\n\
         This may indicate orphaned permissions or missing command registrations.",
        permissions.len(),
        REGISTERED_COMMANDS.len()
    );
}

/// Legacy test using simple string contains (kept for backward compatibility).
/// Prefer test_all_commands_have_default_permissions_toml_parsed for new code.
#[test]
fn test_all_commands_have_default_permissions() {
    let default_toml = include_str!("../permissions/default.toml");

    for cmd in REGISTERED_COMMANDS {
        let permission = format!("allow-{}", cmd.replace('_', "-"));
        assert!(
            default_toml.contains(&permission),
            "Missing permission '{permission}' in default.toml for command '{cmd}'.\n\
             Add '\"{permission}\"' to the [default] permissions array."
        );
    }
}

/// Test that `delete_points` permission is specifically included (regression for #169)
#[test]
fn test_delete_points_permission_exists() {
    let default_toml = include_str!("../permissions/default.toml");
    assert!(
        default_toml.contains("allow-delete-points"),
        "Regression: 'allow-delete-points' permission missing from default.toml (Issue #169)"
    );
}

/// Test that build.rs COMMANDS array matches REGISTERED_COMMANDS.
/// This ensures the autogeneration process covers all commands.
#[test]
fn test_build_rs_commands_match_registered() {
    let build_rs_content = include_str!("../build.rs");

    for cmd in REGISTERED_COMMANDS {
        assert!(
            build_rs_content.contains(&format!("\"{cmd}\"")),
            "Command '{cmd}' is registered but missing from build.rs COMMANDS array.\n\
             Add '\"{cmd}\"' to the COMMANDS array in build.rs to generate its permission file."
        );
    }
}
