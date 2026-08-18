use super::*;

#[test]
fn test_openapi_spec_generation() {
    let openapi = ApiDoc::openapi();
    let json = openapi.to_json().expect("Failed to serialize OpenAPI spec");
    assert!(!json.is_empty(), "OpenAPI spec should not be empty");
    assert!(json.contains("VelesDB API"), "Should contain API title");
    assert!(
        json.contains(env!("CARGO_PKG_VERSION")),
        "Should contain version"
    );
}

#[test]
fn test_openapi_has_all_endpoints() {
    let openapi = ApiDoc::openapi();
    let json = openapi.to_json().expect("Failed to serialize OpenAPI spec");
    assert!(json.contains("/health"), "Should document /health");
    assert!(
        json.contains("/collections"),
        "Should document /collections"
    );
    assert!(
        json.contains(r"/collections/{name}"),
        "Should document collections by name"
    );
    assert!(json.contains("/points"), "Should document points endpoint");
    assert!(
        json.contains(r"/collections/{name}/points/stream"),
        "Should document points stream endpoint"
    );
    assert!(json.contains("/search"), "Should document search endpoint");
    assert!(json.contains("/query"), "Should document /query");
    assert!(json.contains("/aggregate"), "Should document /aggregate");
    assert!(
        json.contains("/query/explain"),
        "Should document /query/explain"
    );
}

#[test]
fn test_openapi_has_all_tags() {
    let openapi = ApiDoc::openapi();
    let json = openapi.to_json().expect("Failed to serialize OpenAPI spec");
    assert!(json.contains("\"health\""), "Should have health tag");
    assert!(
        json.contains("\"collections\""),
        "Should have collections tag"
    );
    assert!(json.contains("\"points\""), "Should have points tag");
    assert!(json.contains("\"search\""), "Should have search tag");
    assert!(json.contains("\"query\""), "Should have query tag");
}

#[test]
fn test_openapi_has_schemas() {
    let openapi = ApiDoc::openapi();
    let json = openapi.to_json().expect("Failed to serialize OpenAPI spec");
    assert!(
        json.contains("CreateCollectionRequest"),
        "Should have CreateCollectionRequest schema"
    );
    assert!(
        json.contains("CollectionResponse"),
        "Should have CollectionResponse schema"
    );
    assert!(
        json.contains("SearchRequest"),
        "Should have SearchRequest schema"
    );
    assert!(
        json.contains("SearchResponse"),
        "Should have SearchResponse schema"
    );
    assert!(
        json.contains("ErrorResponse"),
        "Should have ErrorResponse schema"
    );
}

/// Regenerates `docs/openapi.{json,yaml}` in place instead of only
/// comparing against them. Opt-in via `UPDATE_OPENAPI_SNAPSHOT=1` so that
/// a plain `cargo test` — including the default parallel test threads —
/// never mutates the working tree; see `generate_openapi_spec_files`.
fn update_openapi_snapshot_requested() -> bool {
    std::env::var_os("UPDATE_OPENAPI_SNAPSHOT").is_some()
}

// #[ignore]: excludes this from the general `cargo test --workspace`
// sweep (the "Tests" CI job), which runs with a DIFFERENT feature set
// (persistence,gpu,update-check, no `openapi`/`prometheus`) than the one
// the committed docs/openapi.{json,yaml} were generated under. Run under
// that other feature set, the assert-equal below fails on a real (but
// benign) schema difference -- not staleness, a feature-combination
// mismatch. Only the dedicated `openapi-drift` CI step, which targets
// this test by exact name with `--ignored` under the canonical feature
// set, should ever run it.
#[test]
#[ignore = "run explicitly via the openapi-drift CI job; see comment above"]
fn generate_openapi_spec_files() {
    let openapi = ApiDoc::openapi();
    let json = openapi
        .to_pretty_json()
        .expect("Failed to serialize OpenAPI JSON");
    let yaml = serde_yaml::to_string(&openapi).expect("Failed to serialize OpenAPI YAML");

    // docs/ relative to workspace root
    let docs_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("test: CARGO_MANIFEST_DIR has a parent (crates/)")
        .parent()
        .expect("test: crates/ has a parent (workspace root)")
        .join("docs");
    let json_path = docs_dir.join("openapi.json");
    let yaml_path = docs_dir.join("openapi.yaml");

    if update_openapi_snapshot_requested() {
        std::fs::create_dir_all(&docs_dir).expect("Failed to create docs dir");
        std::fs::write(&json_path, &json).expect("Failed to write openapi.json");
        std::fs::write(&yaml_path, &yaml).expect("Failed to write openapi.yaml");
    } else {
        let committed_json = std::fs::read_to_string(&json_path).expect(
            "Failed to read docs/openapi.json (run with UPDATE_OPENAPI_SNAPSHOT=1 to create it)",
        );
        let committed_yaml = std::fs::read_to_string(&yaml_path).expect(
            "Failed to read docs/openapi.yaml (run with UPDATE_OPENAPI_SNAPSHOT=1 to create it)",
        );
        assert_eq!(
            json, committed_json,
            "docs/openapi.json is stale — rerun with UPDATE_OPENAPI_SNAPSHOT=1 to regenerate"
        );
        assert_eq!(
            yaml, committed_yaml,
            "docs/openapi.yaml is stale — rerun with UPDATE_OPENAPI_SNAPSHOT=1 to regenerate"
        );
    }

    // Verify key endpoints are present
    assert!(
        json.contains("sparse"),
        "OpenAPI spec should contain sparse endpoints"
    );
    assert!(
        json.contains("/graph/edges"),
        "Should contain graph edge endpoints"
    );
    assert!(
        json.contains("/graph/traverse"),
        "Should contain graph traverse endpoint"
    );
    assert!(
        json.contains("/stream/insert"),
        "Should contain stream insert endpoint"
    );
    assert!(
        json.contains("/match"),
        "Should contain match query endpoint"
    );
    assert!(
        json.contains("/search/multi"),
        "Should contain multi-query search endpoint"
    );
}

#[test]
fn test_openapi_has_license() {
    let openapi = ApiDoc::openapi();
    let json = openapi.to_json().expect("Failed to serialize OpenAPI spec");
    assert!(
        json.contains("VelesDB Core License 1.0"),
        "Should have VelesDB Core License 1.0"
    );
}

#[test]
fn test_openapi_pretty_json() {
    let openapi = ApiDoc::openapi();
    let pretty_json = openapi
        .to_pretty_json()
        .expect("Failed to serialize pretty JSON");
    assert!(
        pretty_json.contains('\n'),
        "Pretty JSON should have newlines"
    );
    assert!(
        pretty_json.len() > 1000,
        "OpenAPI spec should be substantial"
    );
}

#[test]
fn test_openapi_has_all_metrics_documented() {
    let openapi = ApiDoc::openapi();
    let json = openapi.to_json().expect("Failed to serialize OpenAPI spec");
    assert!(json.contains("cosine"), "Should document cosine metric");
    assert!(
        json.contains("euclidean"),
        "Should document euclidean metric"
    );
    assert!(json.contains("dot"), "Should document dot product metric");
    assert!(json.contains("hamming"), "Should document hamming metric");
    assert!(json.contains("jaccard"), "Should document jaccard metric");
}

#[test]
fn test_openapi_has_storage_mode_documented() {
    let openapi = ApiDoc::openapi();
    let json = openapi.to_json().expect("Failed to serialize OpenAPI spec");
    assert!(
        json.contains("storage_mode"),
        "Should document storage_mode parameter"
    );
}

#[test]
fn test_openapi_has_search_types_documented() {
    let openapi = ApiDoc::openapi();
    let json = openapi.to_json().expect("Failed to serialize OpenAPI spec");
    assert!(json.contains("text_search"), "Should document text search");
    assert!(
        json.contains("hybrid_search"),
        "Should document hybrid search"
    );
    assert!(json.contains("batch"), "Should document batch search");
}

#[test]
fn test_create_collection_request_default_metric() {
    let json = r#"{"name": "test", "dimension": 128}"#;
    let req: CreateCollectionRequest =
        serde_json::from_str(json).expect("test: valid CreateCollectionRequest JSON");
    assert_eq!(req.metric, "cosine");
}

#[test]
fn test_create_collection_request_with_hamming() {
    let json = r#"{"name": "test", "dimension": 128, "metric": "hamming"}"#;
    let req: CreateCollectionRequest =
        serde_json::from_str(json).expect("test: valid CreateCollectionRequest JSON");
    assert_eq!(req.metric, "hamming");
}

#[test]
fn test_create_collection_request_with_jaccard() {
    let json = r#"{"name": "test", "dimension": 128, "metric": "jaccard"}"#;
    let req: CreateCollectionRequest =
        serde_json::from_str(json).expect("test: valid CreateCollectionRequest JSON");
    assert_eq!(req.metric, "jaccard");
}

#[test]
fn test_create_collection_request_with_storage_mode() {
    let json = r#"{"name": "test", "dimension": 128, "storage_mode": "sq8"}"#;
    let req: CreateCollectionRequest =
        serde_json::from_str(json).expect("test: valid CreateCollectionRequest JSON");
    assert_eq!(req.storage_mode, "sq8");
}

#[test]
fn test_search_request_deserialize() {
    let json = r#"{"vector": [0.1, 0.2, 0.3], "top_k": 5}"#;
    let req: SearchRequest = serde_json::from_str(json).expect("test: valid SearchRequest JSON");
    assert_eq!(req.vector, vec![0.1, 0.2, 0.3]);
    assert_eq!(req.top_k, 5);
}

#[test]
fn test_batch_search_request_deserialize() {
    let json = r#"{"searches": [{"vector": [0.1, 0.2], "top_k": 3}]}"#;
    let req: BatchSearchRequest =
        serde_json::from_str(json).expect("test: valid BatchSearchRequest JSON");
    assert_eq!(req.searches.len(), 1);
    assert_eq!(req.searches[0].top_k, 3);
}

#[test]
fn test_text_search_request_deserialize() {
    let json = r#"{"query": "machine learning", "top_k": 10}"#;
    let req: TextSearchRequest =
        serde_json::from_str(json).expect("test: valid TextSearchRequest JSON");
    assert_eq!(req.query, "machine learning");
    assert_eq!(req.top_k, 10);
}

#[test]
fn test_hybrid_search_request_deserialize() {
    let json = r#"{"vector": [0.1, 0.2], "query": "test", "top_k": 5}"#;
    let req: HybridSearchRequest =
        serde_json::from_str(json).expect("test: valid HybridSearchRequest JSON");
    assert_eq!(req.vector, vec![0.1, 0.2]);
    assert_eq!(req.query, "test");
    assert_eq!(req.top_k, 5);
}

#[test]
fn test_upsert_points_request_deserialize() {
    let json = r#"{"points": [{"id": 1, "vector": [0.1, 0.2]}]}"#;
    let req: UpsertPointsRequest =
        serde_json::from_str(json).expect("test: valid UpsertPointsRequest JSON");
    assert_eq!(req.points.len(), 1);
    assert_eq!(req.points[0].id, 1);
}

#[test]
fn test_collection_response_serialize() {
    let resp = CollectionResponse {
        name: "test".to_string(),
        dimension: 128,
        metric: "cosine".to_string(),
        storage_mode: "full".to_string(),
        point_count: 100,
    };
    let json = serde_json::to_string(&resp).expect("test: serialize CollectionResponse");
    assert!(json.contains("\"name\":\"test\""));
    assert!(json.contains("\"dimension\":128"));
    assert!(json.contains("\"metric\":\"cosine\""));
    assert!(json.contains("\"storage_mode\":\"full\""));
    assert!(json.contains("\"point_count\":100"));
}

#[test]
fn test_search_response_serialize() {
    let resp = SearchResponse {
        results: vec![SearchResultResponse {
            id: 1,
            score: 0.95,
            payload: None,
        }],
    };
    let json = serde_json::to_string(&resp).expect("test: serialize SearchResponse");
    assert!(json.contains("\"results\""));
    // IDs are serialized as strings to prevent JavaScript precision loss (WP-0D).
    assert!(json.contains("\"id\":\"1\""));
}

#[test]
fn test_error_response_serialize() {
    let resp = ErrorResponse {
        error: "Test error".to_string(),
        code: None,
    };
    let json = serde_json::to_string(&resp).expect("test: serialize ErrorResponse");
    assert!(json.contains("\"error\":\"Test error\""));
    // code: None is omitted from JSON output
    assert!(!json.contains("\"code\""));
}

// ========================================================================
// OpenAPI <-> Router structural conformance
// ========================================================================

/// Extracts every `(path_template, HTTP method)` pair declared in the
/// OpenAPI spec. Returns a sorted `Vec` for deterministic assertions.
fn extract_openapi_operations() -> Vec<(String, axum::http::Method)> {
    let openapi = ApiDoc::openapi();
    let mut ops = Vec::new();
    for (path, item) in &openapi.paths.paths {
        if item.get.is_some() {
            ops.push((path.clone(), axum::http::Method::GET));
        }
        if item.post.is_some() {
            ops.push((path.clone(), axum::http::Method::POST));
        }
        if item.put.is_some() {
            ops.push((path.clone(), axum::http::Method::PUT));
        }
        if item.delete.is_some() {
            ops.push((path.clone(), axum::http::Method::DELETE));
        }
        if item.patch.is_some() {
            ops.push((path.clone(), axum::http::Method::PATCH));
        }
    }
    ops.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.as_str().cmp(b.1.as_str())));
    ops
}

/// Converts an OpenAPI path template into a concrete URI by replacing
/// each `{param}` placeholder with a safe dummy value.
fn template_to_uri(template: &str) -> String {
    template
        .replace("{name}", "test_col")
        .replace("{id}", "1")
        .replace("{node_id}", "1")
        .replace("{edge_id}", "1")
        .replace("{label}", "test_label")
        .replace("{property}", "test_prop")
}

/// Creates a minimal [`AppState`] backed by an ephemeral directory.
/// Returns both the state and the `TempDir` guard (must stay alive).
fn create_conformance_state() -> (std::sync::Arc<AppState>, tempfile::TempDir) {
    let dir = tempfile::TempDir::new().expect("test: create temp dir");
    let db = Database::open(dir.path()).expect("test: open database");
    let state = std::sync::Arc::new(AppState {
        db,
        onboarding_metrics: OnboardingMetrics::default(),
        query_limits: parking_lot::RwLock::new(QueryLimits::default()),
        ready: AtomicBool::new(true),
        operational_metrics: velesdb_core::metrics::OperationalMetrics::new_arc(),
        traversal_metrics: Arc::new(velesdb_core::metrics::TraversalMetrics::new()),
        query_duration_histogram: Arc::new(velesdb_core::metrics::DurationHistogram::new()),
    });
    (state, dir)
}

/// Returns `true` when the response is Axum's built-in fallback (route
/// not found), which is a `404` with an empty body. Handler-generated
/// 404s always carry a non-empty JSON body.
async fn is_axum_fallback(resp: axum::http::Response<axum::body::Body>) -> bool {
    if resp.status() != axum::http::StatusCode::NOT_FOUND {
        return false;
    }
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .expect("test: read response body");
    body.is_empty()
}

/// Structural conformance: every `(path, method)` declared in the OpenAPI
/// spec must be reachable through the Axum router (must NOT hit Axum's
/// built-in fallback 404).
#[tokio::test]
async fn test_openapi_routes_match_router() {
    let operations = extract_openapi_operations();
    assert!(
        !operations.is_empty(),
        "OpenAPI spec should declare at least one operation"
    );

    let (state, _dir) = create_conformance_state();
    let router = crate::routes::api_routes().with_state(state);

    let mut failures: Vec<String> = Vec::new();
    for (template, method) in &operations {
        let uri = template_to_uri(template);
        let req = axum::http::Request::builder()
            .method(method)
            .uri(&uri)
            .header("content-type", "application/json")
            .body(axum::body::Body::from("{}"))
            .expect("test: build request");

        let resp = tower::ServiceExt::oneshot(router.clone(), req)
            .await
            .expect("test: send request");

        if is_axum_fallback(resp).await {
            failures.push(format!("{method} {template}"));
        }
    }

    assert!(
        failures.is_empty(),
        "OpenAPI operations with no matching router route:\n  {}",
        failures.join("\n  ")
    );
}
