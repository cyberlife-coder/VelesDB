//! Integration coverage for the Tauri `#[command]` handlers in `commands.rs`,
//! `commands_graph.rs`, and the lifecycle `observer.rs`.
//!
//! The handlers take `AppHandle<R>` + `State<'_, VelesDbState>` arguments that
//! can only exist inside a Tauri `App`. These tests build a real
//! `App<MockRuntime>` (via `tauri::test`), `manage` a `VelesDbState` rooted in a
//! tempdir, and drive each async command directly through
//! `tauri::async_runtime::block_on` — covering both success and error paths
//! (missing collection, wrong collection kind, invalid config) which the
//! existing `*_tests.rs` DTO/serde unit tests do not reach.
//!
//! Requires the `persistence` feature (the core search/streaming paths the
//! commands call are persistence-gated).
#![cfg(feature = "persistence")]
#![allow(clippy::too_many_lines)] // verbose end-to-end command-handler setup

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use tauri::test::{mock_builder, mock_context, noop_assets};
use tauri::{App, AppHandle, Listener, Manager, State};

use tauri::Runtime;
use tauri_plugin_velesdb::commands::{
    add_edge, batch_search, compact_storage, create_collection, create_graph_collection,
    create_metadata_collection, delete_collection, delete_points, enable_streaming, flush,
    get_collection, get_edges, get_guardrails, get_node_degree, get_points, hybrid_search,
    is_empty, list_collections, multi_query_search, scroll_collection, search, search_ids,
    stream_insert, text_search, traverse_graph, update_guardrails, upsert, upsert_metadata,
};
use tauri_plugin_velesdb::observer::TauriObserver;
use tauri_plugin_velesdb::types::{
    AddEdgeRequest, BatchSearchRequest, CreateCollectionRequest, CreateGraphCollectionRequest,
    CreateMetadataCollectionRequest, DeletePointsRequest, EnableStreamingRequest, GetEdgesRequest,
    GetNodeDegreeRequest, GetPointsRequest, GuardrailLimits, HybridSearchRequest,
    IndividualSearchRequest, MetadataPointInput, MultiQuerySearchRequest, PointInput,
    ScrollRequest, SearchRequest, StreamInsertRequest, TextSearchRequest, TraverseGraphRequest,
    UpsertMetadataRequest, UpsertRequest,
};
use tauri_plugin_velesdb::VelesDbState;

const DIM: usize = 4;

/// Builds an `App<MockRuntime>` managing a `VelesDbState` rooted in `dir`.
fn mock_app(dir: &std::path::Path) -> App<tauri::test::MockRuntime> {
    let app = mock_builder()
        .build(mock_context(noop_assets()))
        .expect("build mock app");
    let state = VelesDbState::new(dir.to_path_buf());
    app.manage(state);
    app
}

/// Convenience: run an async command future to completion.
fn run<F: std::future::Future>(fut: F) -> F::Output {
    tauri::async_runtime::block_on(fut)
}

fn handle<R: Runtime>(app: &App<R>) -> AppHandle<R> {
    app.handle().clone()
}

fn st<R: Runtime>(app: &App<R>) -> State<'_, VelesDbState> {
    app.state::<VelesDbState>()
}

fn vector_collection_request(name: &str) -> CreateCollectionRequest {
    serde_json::from_value(serde_json::json!({
        "name": name,
        "dimension": DIM,
    }))
    .expect("deserialize create request")
}

/// Builds the on-wire filter JSON (`{"condition": {"type": "eq", ...}}`) the
/// frontend sends and that `parse_filter` → `Filter::from_json_value` expects.
fn eq_filter(field: &str, value: &str) -> serde_json::Value {
    serde_json::json!({"condition": {"type": "eq", "field": field, "value": value}})
}

// `vector`/`payload` are moved into the `json!` macro; clippy's pedantic
// pass-by-value lint can't see through the macro, so silence it here.
#[allow(clippy::needless_pass_by_value)]
fn point(id: u64, vector: Vec<f32>, payload: serde_json::Value) -> PointInput {
    serde_json::from_value(serde_json::json!({
        "id": id,
        "vector": vector,
        "payload": payload,
    }))
    .expect("deserialize point")
}

/// Creates a vector collection and upserts four text-rich points so vector,
/// text (BM25, populated automatically from string payloads), and hybrid
/// searches all have data to return.
fn seed_vector_collection(app: &App<tauri::test::MockRuntime>, name: &str) {
    let created = run(create_collection(
        handle(app),
        st(app),
        vector_collection_request(name),
    ))
    .expect("create_collection");
    assert_eq!(created.name, name);
    assert_eq!(created.dimension, DIM);

    let points = vec![
        point(
            1,
            vec![1.0, 0.0, 0.0, 0.0],
            serde_json::json!({"title": "rust programming language", "category": "tech"}),
        ),
        point(
            2,
            vec![0.0, 1.0, 0.0, 0.0],
            serde_json::json!({"title": "python programming tutorial", "category": "tech"}),
        ),
        point(
            3,
            vec![0.0, 0.0, 1.0, 0.0],
            serde_json::json!({"title": "football world cup", "category": "sports"}),
        ),
        point(
            4,
            vec![0.5, 0.5, 0.0, 0.0],
            serde_json::json!({"title": "rust web framework", "category": "tech"}),
        ),
    ];
    let inserted = run(upsert(
        handle(app),
        st(app),
        UpsertRequest {
            collection: name.to_string(),
            points,
        },
    ))
    .expect("upsert");
    assert_eq!(inserted, 4);
}

// ===========================================================================
// Collection lifecycle commands
// ===========================================================================

#[test]
fn create_list_get_delete_collection_roundtrip() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app = mock_app(dir.path());

    seed_vector_collection(&app, "docs");

    // list_collections sees it with the right shape.
    let listed = run(list_collections(handle(&app), st(&app))).expect("list");
    assert!(listed.iter().any(|c| c.name == "docs" && c.count == 4));

    // get_collection returns matching info.
    let info = run(get_collection(handle(&app), st(&app), "docs".to_string())).expect("get");
    assert_eq!(info.dimension, DIM);
    assert_eq!(info.count, 4);

    // is_empty is false after seeding.
    assert!(!run(is_empty(handle(&app), st(&app), "docs".to_string())).expect("is_empty"));

    // flush + compact_storage succeed on a real collection.
    run(flush(handle(&app), st(&app), "docs".to_string())).expect("flush");
    let _freed =
        run(compact_storage(handle(&app), st(&app), "docs".to_string())).expect("compact_storage");

    // delete_collection removes it; subsequent get_collection errors.
    run(delete_collection(
        handle(&app),
        st(&app),
        "docs".to_string(),
    ))
    .expect("delete");
    let err = run(get_collection(handle(&app), st(&app), "docs".to_string()))
        .expect_err("missing collection should error");
    assert_eq!(err.code, "VELES-002");
}

#[test]
fn create_collection_with_advanced_params_validates() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app = mock_app(dir.path());

    // hnswM triggers the advanced-params path (build_hnsw_params + validate).
    let req: CreateCollectionRequest = serde_json::from_value(serde_json::json!({
        "name": "tuned",
        "dimension": DIM,
        "hnswM": 32,
        "hnswEfConstruction": 200,
    }))
    .expect("deserialize");
    let info = run(create_collection(handle(&app), st(&app), req)).expect("create tuned");
    assert_eq!(info.name, "tuned");

    // An out-of-range alpha is rejected by hnsw_params.validate().
    let bad: CreateCollectionRequest = serde_json::from_value(serde_json::json!({
        "name": "bad",
        "dimension": DIM,
        "hnswAlpha": 0.1,
    }))
    .expect("deserialize");
    assert!(run(create_collection(handle(&app), st(&app), bad)).is_err());
}

#[test]
fn create_collection_invalid_metric_errors() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app = mock_app(dir.path());
    let mut req = vector_collection_request("x");
    req.metric = "not_a_metric".to_string();
    let err = run(create_collection(handle(&app), st(&app), req)).expect_err("bad metric");
    assert_eq!(err.code, "INVALID_CONFIG");
}

#[test]
fn create_metadata_collection_and_upsert_metadata() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app = mock_app(dir.path());

    // create_metadata_collection reports the metadata-only shape.
    let info = run(create_metadata_collection(
        handle(&app),
        st(&app),
        CreateMetadataCollectionRequest {
            name: "meta".to_string(),
        },
    ))
    .expect("create metadata");
    assert_eq!(info.storage_mode, "metadata_only");

    let m_point: MetadataPointInput = serde_json::from_value(serde_json::json!({
        "id": 1,
        "payload": {"name": "alice"},
    }))
    .expect("deserialize metadata point");

    // upsert_metadata goes through `require_collection`, which only accepts
    // vector collections — so a metadata-only target is rejected (error path).
    let rejected = run(upsert_metadata(
        handle(&app),
        st(&app),
        UpsertMetadataRequest {
            collection: "meta".to_string(),
            points: vec![m_point],
        },
    ))
    .expect_err("metadata-only collection is not a vector collection");
    assert_eq!(rejected.code, "INVALID_CONFIG");

    // The success path: metadata-only points (no vector) into a vector
    // collection exercises the `coll.upsert_metadata` branch.
    seed_vector_collection(&app, "docs");
    let m_point2: MetadataPointInput = serde_json::from_value(serde_json::json!({
        "id": 99,
        "payload": {"name": "bob"},
    }))
    .expect("deserialize metadata point");
    let n = run(upsert_metadata(
        handle(&app),
        st(&app),
        UpsertMetadataRequest {
            collection: "docs".to_string(),
            points: vec![m_point2],
        },
    ))
    .expect("upsert_metadata into vector collection");
    assert_eq!(n, 1);
}

// ===========================================================================
// Point + search commands (success + error paths)
// ===========================================================================

#[test]
fn get_and_delete_points() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app = mock_app(dir.path());
    seed_vector_collection(&app, "docs");

    let fetched = run(get_points(
        handle(&app),
        st(&app),
        GetPointsRequest {
            collection: "docs".to_string(),
            ids: vec![1, 999],
        },
    ))
    .expect("get_points");
    assert_eq!(fetched.len(), 2);
    assert!(fetched[0].is_some(), "id 1 exists");
    assert!(fetched[1].is_none(), "id 999 missing");

    run(delete_points(
        handle(&app),
        st(&app),
        DeletePointsRequest {
            collection: "docs".to_string(),
            ids: vec![1],
        },
    ))
    .expect("delete_points");

    let after = run(get_points(
        handle(&app),
        st(&app),
        GetPointsRequest {
            collection: "docs".to_string(),
            ids: vec![1],
        },
    ))
    .expect("get_points after delete");
    assert!(after[0].is_none(), "id 1 deleted");
}

#[test]
fn search_plain_filtered_and_quality() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app = mock_app(dir.path());
    seed_vector_collection(&app, "docs");

    // Plain vector search.
    let plain = run(search(
        handle(&app),
        st(&app),
        SearchRequest {
            collection: "docs".to_string(),
            vector: vec![1.0, 0.0, 0.0, 0.0],
            top_k: 3,
            filter: None,
            quality: None,
        },
    ))
    .expect("search plain");
    assert!(!plain.results.is_empty());

    // Quality mode (persistence path: dispatch_quality_search → search_with_quality).
    let quality = run(search(
        handle(&app),
        st(&app),
        SearchRequest {
            collection: "docs".to_string(),
            vector: vec![1.0, 0.0, 0.0, 0.0],
            top_k: 2,
            filter: None,
            quality: Some("accurate".to_string()),
        },
    ))
    .expect("search quality");
    assert!(!quality.results.is_empty());

    // Filtered search (search_with_filter path).
    let filtered = run(search(
        handle(&app),
        st(&app),
        SearchRequest {
            collection: "docs".to_string(),
            vector: vec![0.5, 0.5, 0.0, 0.0],
            top_k: 10,
            filter: Some(eq_filter("category", "sports")),
            quality: None,
        },
    ))
    .expect("search filtered");
    for r in &filtered.results {
        assert_eq!(r.id, 3, "only the sports doc matches the filter");
    }
}

#[test]
fn search_missing_collection_and_bad_quality_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app = mock_app(dir.path());
    seed_vector_collection(&app, "docs");

    let missing = run(search(
        handle(&app),
        st(&app),
        SearchRequest {
            collection: "ghost".to_string(),
            vector: vec![1.0, 0.0, 0.0, 0.0],
            top_k: 1,
            filter: None,
            quality: None,
        },
    ))
    .expect_err("missing collection");
    assert_eq!(missing.code, "VELES-002");

    let bad_quality = run(search(
        handle(&app),
        st(&app),
        SearchRequest {
            collection: "docs".to_string(),
            vector: vec![1.0, 0.0, 0.0, 0.0],
            top_k: 1,
            filter: None,
            quality: Some("nonsense".to_string()),
        },
    ))
    .expect_err("invalid quality mode");
    assert_eq!(bad_quality.code, "INVALID_CONFIG");
}

#[test]
fn search_ids_optimized_and_filtered_paths() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app = mock_app(dir.path());
    seed_vector_collection(&app, "docs");

    // No filter, no quality → optimized search_ids path.
    let ids = run(search_ids(
        handle(&app),
        st(&app),
        SearchRequest {
            collection: "docs".to_string(),
            vector: vec![1.0, 0.0, 0.0, 0.0],
            top_k: 2,
            filter: None,
            quality: None,
        },
    ))
    .expect("search_ids optimized");
    assert!(!ids.is_empty());

    // Filter present → falls back to full search then project_search_ids.
    let filtered_ids = run(search_ids(
        handle(&app),
        st(&app),
        SearchRequest {
            collection: "docs".to_string(),
            vector: vec![0.0, 0.0, 1.0, 0.0],
            top_k: 10,
            filter: Some(eq_filter("category", "sports")),
            quality: None,
        },
    ))
    .expect("search_ids filtered");
    for hit in &filtered_ids {
        assert_eq!(hit.id, 3);
    }
}

#[test]
fn batch_search_bulk_and_per_query_quality() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app = mock_app(dir.path());
    seed_vector_collection(&app, "docs");

    let plain_searches: Vec<IndividualSearchRequest> = serde_json::from_value(serde_json::json!([
        {"vector": [1.0, 0.0, 0.0, 0.0], "topK": 2},
        {"vector": [0.0, 1.0, 0.0, 0.0], "topK": 1},
    ]))
    .expect("deserialize searches");
    let bulk = run(batch_search(
        handle(&app),
        st(&app),
        BatchSearchRequest {
            collection: "docs".to_string(),
            searches: plain_searches,
        },
    ))
    .expect("batch_search bulk");
    assert_eq!(bulk.len(), 2);
    assert!(bulk[1].results.len() <= 1, "second query honors topK=1");

    // A per-query quality flips to the per-query dispatch branch.
    let quality_searches: Vec<IndividualSearchRequest> =
        serde_json::from_value(serde_json::json!([
            {"vector": [1.0, 0.0, 0.0, 0.0], "topK": 2, "quality": "fast"},
        ]))
        .expect("deserialize quality searches");
    let per_query = run(batch_search(
        handle(&app),
        st(&app),
        BatchSearchRequest {
            collection: "docs".to_string(),
            searches: quality_searches,
        },
    ))
    .expect("batch_search per-query");
    assert_eq!(per_query.len(), 1);
    assert!(
        !per_query[0].results.is_empty(),
        "per-query quality search must return results"
    );
    // Query [1.0, 0.0, 0.0, 0.0] is the exact vector of seeded point id=1,
    // so the quality dispatch must rank it first.
    assert_eq!(
        per_query[0].results[0].id, 1,
        "fast-quality dispatch must return the nearest neighbor (id=1)"
    );
}

#[test]
fn text_and_hybrid_search_filtered_and_unfiltered() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app = mock_app(dir.path());
    seed_vector_collection(&app, "docs");

    // Unfiltered BM25 text search finds the two rust docs.
    let text = run(text_search(
        handle(&app),
        st(&app),
        TextSearchRequest {
            collection: "docs".to_string(),
            query: "rust".to_string(),
            top_k: 10,
            filter: None,
        },
    ))
    .expect("text_search");
    let text_ids: Vec<u64> = text.results.iter().map(|r| r.id).collect();
    assert!(text_ids.contains(&1) && text_ids.contains(&4));

    // Filtered text search restricts to tech.
    let text_filtered = run(text_search(
        handle(&app),
        st(&app),
        TextSearchRequest {
            collection: "docs".to_string(),
            query: "rust".to_string(),
            top_k: 10,
            filter: Some(eq_filter("category", "tech")),
        },
    ))
    .expect("text_search filtered");
    assert!(
        !text_filtered.results.is_empty(),
        "filter must not drop all matches"
    );
    for r in &text_filtered.results {
        assert_eq!(
            r.payload
                .as_ref()
                .and_then(|p| p.get("category"))
                .and_then(|v| v.as_str()),
            Some("tech"),
            "filtered text search leaked a non-tech doc: id={}",
            r.id,
        );
    }

    // Hybrid search (unfiltered + filtered branches).
    let hybrid = run(hybrid_search(
        handle(&app),
        st(&app),
        HybridSearchRequest {
            collection: "docs".to_string(),
            vector: vec![1.0, 0.0, 0.0, 0.0],
            query: "rust".to_string(),
            top_k: 5,
            vector_weight: 0.5,
            filter: None,
        },
    ))
    .expect("hybrid_search");
    assert!(!hybrid.results.is_empty());

    let hybrid_filtered = run(hybrid_search(
        handle(&app),
        st(&app),
        HybridSearchRequest {
            collection: "docs".to_string(),
            vector: vec![1.0, 0.0, 0.0, 0.0],
            query: "rust".to_string(),
            top_k: 5,
            vector_weight: 0.7,
            filter: Some(eq_filter("category", "tech")),
        },
    ))
    .expect("hybrid_search filtered");
    assert!(
        !hybrid_filtered.results.is_empty(),
        "filter must not drop all matches"
    );
    for r in &hybrid_filtered.results {
        assert_eq!(
            r.payload
                .as_ref()
                .and_then(|p| p.get("category"))
                .and_then(|v| v.as_str()),
            Some("tech"),
            "filtered hybrid search leaked a non-tech doc: id={}",
            r.id,
        );
    }
}

#[test]
fn multi_query_search_fuses_results() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app = mock_app(dir.path());
    seed_vector_collection(&app, "docs");

    let response = run(multi_query_search(
        handle(&app),
        st(&app),
        MultiQuerySearchRequest {
            collection: "docs".to_string(),
            vectors: vec![vec![1.0, 0.0, 0.0, 0.0], vec![0.0, 1.0, 0.0, 0.0]],
            top_k: 3,
            fusion: "rrf".to_string(),
            fusion_params: Some(serde_json::json!({"k": 30})),
            filter: None,
        },
    ))
    .expect("multi_query_search");
    assert!(!response.results.is_empty());

    // Unknown fusion strategy is a config error.
    let bad = run(multi_query_search(
        handle(&app),
        st(&app),
        MultiQuerySearchRequest {
            collection: "docs".to_string(),
            vectors: vec![vec![1.0, 0.0, 0.0, 0.0]],
            top_k: 1,
            fusion: "nope".to_string(),
            fusion_params: None,
            filter: None,
        },
    ))
    .expect_err("bad fusion");
    assert_eq!(bad.code, "INVALID_CONFIG");
}

#[test]
fn scroll_collection_paginates() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app = mock_app(dir.path());
    seed_vector_collection(&app, "docs");

    let first = run(scroll_collection(
        handle(&app),
        st(&app),
        ScrollRequest {
            collection: "docs".to_string(),
            cursor: None,
            batch_size: 2,
            filter: None,
        },
    ))
    .expect("scroll first page");
    assert_eq!(
        first.points.len(),
        2,
        "first page must be full at batch_size=2"
    );
    assert_eq!(first.points[0].id, 1);
    assert_eq!(first.points[1].id, 2);
    assert_eq!(first.next_cursor, Some(2));

    // Filtered scroll only returns the sports doc.
    let filtered = run(scroll_collection(
        handle(&app),
        st(&app),
        ScrollRequest {
            collection: "docs".to_string(),
            cursor: None,
            batch_size: 100,
            filter: Some(eq_filter("category", "sports")),
        },
    ))
    .expect("scroll filtered");
    assert_eq!(filtered.points.len(), 1, "exactly one sports doc");
    assert_eq!(filtered.points[0].id, 3);
}

// ===========================================================================
// Guardrails commands
// ===========================================================================

#[test]
fn update_then_get_guardrails() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app = mock_app(dir.path());
    seed_vector_collection(&app, "docs");

    let limits = GuardrailLimits {
        max_depth: 7,
        max_cardinality: 1234,
        memory_limit_bytes: 4096,
        timeout_ms: 500,
        rate_limit_qps: 42,
        circuit_failure_threshold: 3,
        circuit_recovery_seconds: 30,
    };
    run(update_guardrails(handle(&app), st(&app), limits)).expect("update_guardrails");

    let got =
        run(get_guardrails(handle(&app), st(&app), "docs".to_string())).expect("get_guardrails");
    assert_eq!(got.max_depth, 7);
    assert_eq!(got.timeout_ms, 500);
}

// ===========================================================================
// Streaming commands (persistence-only)
// ===========================================================================

#[test]
fn enable_streaming_then_stream_insert() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app = mock_app(dir.path());
    seed_vector_collection(&app, "docs");

    // stream_insert before enabling streaming errors (backpressure / not configured).
    let not_ready = run(stream_insert(
        handle(&app),
        st(&app),
        StreamInsertRequest {
            collection: "docs".to_string(),
            points: vec![point(10, vec![0.1, 0.2, 0.3, 0.4], serde_json::json!({}))],
        },
    ));
    assert!(
        not_ready.is_err(),
        "stream_insert without enable should error"
    );

    run(enable_streaming(
        handle(&app),
        st(&app),
        EnableStreamingRequest {
            collection: "docs".to_string(),
            buffer_size: Some(256),
            batch_size: Some(8),
            flush_interval_ms: Some(10),
        },
    ))
    .expect("enable_streaming");

    let inserted = run(stream_insert(
        handle(&app),
        st(&app),
        StreamInsertRequest {
            collection: "docs".to_string(),
            points: vec![
                point(11, vec![0.1, 0.2, 0.3, 0.4], serde_json::json!({"k": "v"})),
                point(12, vec![0.4, 0.3, 0.2, 0.1], serde_json::json!({"k": "w"})),
            ],
        },
    ))
    .expect("stream_insert after enable");
    assert_eq!(inserted, 2);
}

// ===========================================================================
// Knowledge graph commands
// ===========================================================================

#[test]
fn graph_create_add_edges_traverse_degree() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app = mock_app(dir.path());

    let info = run(create_graph_collection(
        handle(&app),
        st(&app),
        CreateGraphCollectionRequest {
            name: "kg".to_string(),
            dimension: None,
            metric: "cosine".to_string(),
            graph_schema: None,
        },
    ))
    .expect("create_graph_collection");
    assert_eq!(info.storage_mode, "graph");

    // add_edge requires both endpoints to have a stored node payload
    // (#1442) — store nodes 10, 20, 30 before creating the edges below.
    st(&app)
        .with_db(|db| {
            let coll = db.get_graph_collection("kg").expect("kg collection");
            for node_id in [10, 20, 30] {
                coll.upsert_node_payload(node_id, &serde_json::json!({}))
                    .expect("store node payload");
            }
            Ok(())
        })
        .expect("seed graph nodes");

    // Add a single edge with properties.
    run(add_edge(
        handle(&app),
        st(&app),
        AddEdgeRequest {
            collection: "kg".to_string(),
            id: 1,
            source: 10,
            target: 20,
            label: "KNOWS".to_string(),
            properties: Some(serde_json::json!({"weight": 0.9})),
        },
    ))
    .expect("add_edge");
    run(add_edge(
        handle(&app),
        st(&app),
        AddEdgeRequest {
            collection: "kg".to_string(),
            id: 2,
            source: 20,
            target: 30,
            label: "KNOWS".to_string(),
            properties: None,
        },
    ))
    .expect("add_edge 2");

    // get_edges by source.
    let outgoing = run(get_edges(
        handle(&app),
        st(&app),
        GetEdgesRequest {
            collection: "kg".to_string(),
            label: None,
            source: Some(10),
            target: None,
        },
    ))
    .expect("get_edges source");
    assert_eq!(outgoing.len(), 1);
    assert_eq!(outgoing[0].target, 20);

    // get_edges by label.
    let by_label = run(get_edges(
        handle(&app),
        st(&app),
        GetEdgesRequest {
            collection: "kg".to_string(),
            label: Some("KNOWS".to_string()),
            source: None,
            target: None,
        },
    ))
    .expect("get_edges label");
    assert_eq!(by_label.len(), 2);

    // Node degree of node 20: one in (10->20), one out (20->30).
    let degree = run(get_node_degree(
        handle(&app),
        st(&app),
        GetNodeDegreeRequest {
            collection: "kg".to_string(),
            node_id: 20,
        },
    ))
    .expect("get_node_degree");
    assert_eq!(degree.in_degree, 1);
    assert_eq!(degree.out_degree, 1);

    // BFS traversal from node 10 reaches 20 (and 30 at depth 2).
    let bfs = run(traverse_graph(
        handle(&app),
        st(&app),
        TraverseGraphRequest {
            collection: "kg".to_string(),
            source: 10,
            max_depth: 2,
            limit: 10,
            algorithm: "bfs".to_string(),
            rel_types: None,
        },
    ))
    .expect("traverse_graph bfs");
    let reached: Vec<u64> = bfs.iter().map(|t| t.target_id).collect();
    assert!(reached.contains(&20));

    // DFS branch.
    let dfs = run(traverse_graph(
        handle(&app),
        st(&app),
        TraverseGraphRequest {
            collection: "kg".to_string(),
            source: 10,
            max_depth: 2,
            limit: 10,
            algorithm: "dfs".to_string(),
            rel_types: Some(vec!["KNOWS".to_string()]),
        },
    ))
    .expect("traverse_graph dfs");
    assert!(!dfs.is_empty());
}

#[test]
fn graph_create_with_embeddings_and_missing_collection_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app = mock_app(dir.path());

    // dimension set → create_graph_collection_with_embeddings branch.
    let info = run(create_graph_collection(
        handle(&app),
        st(&app),
        CreateGraphCollectionRequest {
            name: "kg_emb".to_string(),
            dimension: Some(DIM),
            metric: "cosine".to_string(),
            graph_schema: None,
        },
    ))
    .expect("create_graph_collection with embeddings");
    assert_eq!(info.dimension, DIM);

    // add_edge on a non-existent graph collection errors.
    let err = run(add_edge(
        handle(&app),
        st(&app),
        AddEdgeRequest {
            collection: "ghost".to_string(),
            id: 1,
            source: 1,
            target: 2,
            label: "X".to_string(),
            properties: None,
        },
    ))
    .expect_err("missing graph collection");
    assert_eq!(err.code, "VELES-002");
}

#[test]
fn create_graph_collection_invalid_schema_errors() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app = mock_app(dir.path());
    let err = run(create_graph_collection(
        handle(&app),
        st(&app),
        CreateGraphCollectionRequest {
            name: "kg_bad".to_string(),
            dimension: None,
            metric: "cosine".to_string(),
            graph_schema: Some(serde_json::json!("not-a-schema-object")),
        },
    ))
    .expect_err("invalid schema");
    assert_eq!(err.code, "INVALID_CONFIG");
}

// ===========================================================================
// Lifecycle observer (observer.rs)
// ===========================================================================

#[test]
fn tauri_observer_forwards_lifecycle_hooks_to_app() {
    // A managed state built with `new_with_observer` and a `TauriObserver`
    // exercises the observer's `on_collection_created` / `on_collection_deleted`
    // hooks (they emit Tauri events through the app handle) end-to-end via the
    // create/delete command path.
    let dir = tempfile::tempdir().expect("tempdir");
    let app = mock_builder()
        .build(mock_context(noop_assets()))
        .expect("build app");

    // Register listeners before managing state so events fired during
    // create/delete are counted. Tauri dispatches listeners synchronously on
    // the emitting thread, so the counters are safe to read after block_on.
    let created = Arc::new(AtomicUsize::new(0));
    let deleted = Arc::new(AtomicUsize::new(0));
    let c = Arc::clone(&created);
    app.handle().listen(
        tauri_plugin_velesdb::events::event_names::COLLECTION_CREATED,
        move |_| {
            c.fetch_add(1, Ordering::SeqCst);
        },
    );
    let d = Arc::clone(&deleted);
    app.handle().listen(
        tauri_plugin_velesdb::events::event_names::COLLECTION_DELETED,
        move |_| {
            d.fetch_add(1, Ordering::SeqCst);
        },
    );

    let observer = Arc::new(TauriObserver::new(app.handle().clone()));
    let state = VelesDbState::new_with_observer(dir.path().to_path_buf(), observer);
    app.manage(state);

    // create then delete drives both observer hooks (emit_collection_created /
    // emit_collection_deleted) without panicking on the mock runtime.
    let info = run(create_collection(
        handle(&app),
        st(&app),
        vector_collection_request("observed"),
    ))
    .expect("create observed");
    assert_eq!(info.name, "observed");
    assert_eq!(
        created.load(Ordering::SeqCst),
        1,
        "TauriObserver did not emit collection-created"
    );

    run(delete_collection(
        handle(&app),
        st(&app),
        "observed".to_string(),
    ))
    .expect("delete observed");
    assert_eq!(
        deleted.load(Ordering::SeqCst),
        1,
        "TauriObserver did not emit collection-deleted"
    );
}
