//! End-to-end coverage for item P: a [`DatabaseObserver`] injected when the
//! server's database is opened must receive the lifecycle *notify* hooks as
//! real HTTP requests flow through the handlers.
//!
//! Each assertion proves a distinct piece of wiring is live:
//! - `POST /collections`              → `on_collection_created`
//! - `POST /collections/{name}/points` → `on_upsert` (`points/mod.rs`)
//! - `POST /collections/{name}/search` → `on_query`  (`search/pipeline.rs`)
//! - `DELETE /collections/{name}`     → `on_collection_deleted`

mod common;

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use common::create_test_app_with_observer;
use serde_json::{json, Value};
use tempfile::TempDir;
use tower::ServiceExt;
use velesdb_core::collection::CollectionType;
use velesdb_core::DatabaseObserver;

const COLLECTION: &str = "observed";
const DIM: usize = 4;
const QUERY: [f32; DIM] = [1.0, 0.5, 0.25, 0.1];

/// Counts each notify hook independently. Mirrors the Tauri-side counting
/// observer; uses `AtomicUsize` because callbacks may fire from any thread.
#[derive(Default)]
struct CountingObserver {
    created: AtomicUsize,
    deleted: AtomicUsize,
    upsert: AtomicUsize,
    query: AtomicUsize,
}

impl DatabaseObserver for CountingObserver {
    fn on_collection_created(&self, _name: &str, _kind: &CollectionType) {
        self.created.fetch_add(1, Ordering::SeqCst);
    }
    fn on_collection_deleted(&self, _name: &str) {
        self.deleted.fetch_add(1, Ordering::SeqCst);
    }
    fn on_upsert(&self, _collection: &str, _point_count: usize) {
        self.upsert.fetch_add(1, Ordering::SeqCst);
    }
    fn on_query(&self, _collection: &str, _duration_us: u64) {
        self.query.fetch_add(1, Ordering::SeqCst);
    }
}

async fn post(app: &axum::Router, uri: &str, body: Value) -> StatusCode {
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("Content-Type", "application/json")
                .body(Body::from(body.to_string()))
                .expect("test: build POST request"),
        )
        .await
        .expect("test: POST request")
        .status()
}

async fn get(app: &axum::Router, uri: &str) -> StatusCode {
    app.clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(uri)
                .body(Body::empty())
                .expect("test: build GET request"),
        )
        .await
        .expect("test: GET request")
        .status()
}

async fn delete(app: &axum::Router, uri: &str) -> StatusCode {
    app.clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(uri)
                .body(Body::empty())
                .expect("test: build DELETE request"),
        )
        .await
        .expect("test: DELETE request")
        .status()
}

#[tokio::test]
async fn observer_receives_full_lifecycle() {
    let dir = TempDir::new().expect("test: dir");
    let observer = Arc::new(CountingObserver::default());
    let app = create_test_app_with_observer(&dir, observer.clone());

    // create → on_collection_created
    let status = post(
        &app,
        "/collections",
        json!({"name": COLLECTION, "dimension": DIM, "metric": "cosine"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "create");
    assert_eq!(
        observer.created.load(Ordering::SeqCst),
        1,
        "on_collection_created"
    );

    // upsert → on_upsert (points/mod.rs notify_upsert)
    let status = post(
        &app,
        &format!("/collections/{COLLECTION}/points"),
        json!({"points": [{"id": 1, "vector": QUERY, "payload": {"k": "v"}}]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "upsert");
    assert!(observer.upsert.load(Ordering::SeqCst) >= 1, "on_upsert");

    // search → on_query (search/pipeline.rs notify_query)
    let status = post(
        &app,
        &format!("/collections/{COLLECTION}/search"),
        json!({"vector": QUERY, "top_k": 1}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "search");
    assert!(observer.query.load(Ordering::SeqCst) >= 1, "on_query");

    // delete → on_collection_deleted
    let status = delete(&app, &format!("/collections/{COLLECTION}")).await;
    assert_eq!(status, StatusCode::OK, "delete");
    assert_eq!(
        observer.deleted.load(Ordering::SeqCst),
        1,
        "on_collection_deleted"
    );
}

/// Regression test for a double-count bug: `Database::execute_query` fires
/// `on_query` once internally (core-invoked telemetry). The `/query` REST
/// handler must not *also* invoke the deprecated `notify_query` shim after
/// calling `execute_query`, or every `VelesQL` request would tally twice for
/// any registered `DatabaseObserver` (RBAC/audit/usage billing). Unlike
/// `observer_receives_full_lifecycle` above (which asserts `>= 1` for the
/// `/search` REST pipeline, a path that never routes through
/// `execute_query`), this test asserts an exact count for both `/query`
/// dispatch branches so a reintroduced double-fire fails the test.
#[tokio::test]
async fn query_endpoint_fires_on_query_exactly_once_per_dispatch_branch() {
    let dir = TempDir::new().expect("test: dir");
    let observer = Arc::new(CountingObserver::default());
    let app = create_test_app_with_observer(&dir, observer.clone());

    let status = post(
        &app,
        "/collections",
        json!({"name": COLLECTION, "dimension": DIM, "metric": "cosine"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "create");

    // SELECT via /query exercises `execute_standard_query`.
    let status = post(
        &app,
        "/query",
        json!({
            "query": format!("SELECT * FROM {COLLECTION} WHERE vector NEAR $v LIMIT 1"),
            "params": {"v": QUERY}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "select via /query");
    assert_eq!(
        observer.query.load(Ordering::SeqCst),
        1,
        "on_query must fire exactly once for a SELECT dispatched via /query"
    );

    // SHOW COLLECTIONS via /query exercises the AST-routed
    // `execute_mutation_query` branch (introspection).
    let status = post(&app, "/query", json!({"query": "SHOW COLLECTIONS"})).await;
    assert_eq!(status, StatusCode::OK, "show collections via /query");
    assert_eq!(
        observer.query.load(Ordering::SeqCst),
        2,
        "on_query must fire exactly once more for an introspection query dispatched via /query"
    );
}

/// Refuses every read at the control-plane gate — models an unauthorized
/// principal. Writes are unaffected: `on_query_request` gates reads only.
struct DenyingReadObserver;
impl DatabaseObserver for DenyingReadObserver {
    fn on_query_request(
        &self,
        _ctx: &velesdb_core::observer::QueryAccessContext,
    ) -> velesdb_core::Result<velesdb_core::observer::AccessDecision> {
        Ok(velesdb_core::observer::AccessDecision::Deny(
            velesdb_core::Error::Query("read denied by governance policy".to_string()),
        ))
    }
}

/// End-to-end proof that the read-path gate (CORE-1/CORE-2) is enforced through
/// the HTTP layer: a `Deny` observer must refuse `/search`, `/search/text`, and
/// `/search/hybrid` — none may return `200 OK` — while writes still succeed
/// (they are not read-gated). This closes the gap where the REST search
/// handlers called a detached `VectorCollection` handle that bypassed the gate.
#[tokio::test]
async fn read_gate_denies_rest_search_end_to_end() {
    let dir = TempDir::new().expect("test: dir");
    let app = create_test_app_with_observer(&dir, Arc::new(DenyingReadObserver));

    // Writes are not read-gated: create + upsert must still succeed.
    let status = post(
        &app,
        "/collections",
        json!({"name": COLLECTION, "dimension": DIM, "metric": "cosine"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "create must not be read-gated");

    let status = post(
        &app,
        &format!("/collections/{COLLECTION}/points"),
        json!({"points": [{"id": 1, "vector": QUERY, "payload": {"k": "v"}}]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "upsert must not be read-gated");

    // Dense /search is refused end-to-end (HTTP → gated_search → observer Deny).
    let status = post(
        &app,
        &format!("/collections/{COLLECTION}/search"),
        json!({"vector": QUERY, "top_k": 1}),
    )
    .await;
    assert!(
        !status.is_success(),
        "denied dense read must not return success, got {status}"
    );

    // Text search is refused.
    let status = post(
        &app,
        &format!("/collections/{COLLECTION}/search/text"),
        json!({"query": "hello", "top_k": 1}),
    )
    .await;
    assert!(
        !status.is_success(),
        "denied text read must not return success, got {status}"
    );

    // Hybrid (dense + BM25) search is refused.
    let status = post(
        &app,
        &format!("/collections/{COLLECTION}/search/hybrid"),
        json!({"vector": QUERY, "query": "hello", "top_k": 1, "vector_weight": 0.5}),
    )
    .await;
    assert!(
        !status.is_success(),
        "denied hybrid read must not return success, got {status}"
    );
}

/// Denies reads while `deny` is set, with an error core classifies as the
/// engine's (`VELES-013`): what a policy engine chooses to answer a denial
/// with is its own business, and must not decide whether it counts.
#[derive(Default)]
struct SwitchableDenyObserver {
    deny: AtomicBool,
}
impl DatabaseObserver for SwitchableDenyObserver {
    fn on_query_request(
        &self,
        _ctx: &velesdb_core::observer::QueryAccessContext,
    ) -> velesdb_core::Result<velesdb_core::observer::AccessDecision> {
        if self.deny.load(Ordering::SeqCst) {
            return Ok(velesdb_core::observer::AccessDecision::Deny(
                velesdb_core::Error::Internal("policy engine refused the read".to_string()),
            ));
        }
        Ok(velesdb_core::observer::AccessDecision::Allow)
    }
}

/// A read-gate denial is the policy's answer, not a sign the collection is
/// unhealthy: however many reads `/search`, `/search/ids`, `/search/text`
/// and `/search/hybrid` refuse, the circuit breaker stays closed and the
/// collection answers the next allowed read.
#[tokio::test]
async fn read_gate_denials_never_open_the_circuit_breaker() {
    let dir = TempDir::new().expect("test: dir");
    let observer = Arc::new(SwitchableDenyObserver::default());
    let app =
        create_test_app_with_observer(&dir, Arc::clone(&observer) as Arc<dyn DatabaseObserver>);
    let status = post(
        &app,
        "/collections",
        json!({"name": COLLECTION, "dimension": DIM, "metric": "cosine"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let status = post(
        &app,
        &format!("/collections/{COLLECTION}/points"),
        json!({"points": [{"id": 1, "vector": QUERY, "payload": {"k": "hello"}}]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    observer.deny.store(true, Ordering::SeqCst);
    let reads = [
        ("search", json!({"vector": QUERY, "top_k": 1})),
        ("search/ids", json!({"vector": QUERY, "top_k": 1})),
        ("search/text", json!({"query": "hello", "top_k": 1})),
        (
            "search/hybrid",
            json!({"vector": QUERY, "query": "hello", "top_k": 1, "vector_weight": 0.5}),
        ),
    ];
    // Ten denials per endpoint: twice the default threshold of five.
    for (path, body) in &reads {
        for _ in 0..10 {
            let status = post(
                &app,
                &format!("/collections/{COLLECTION}/{path}"),
                body.clone(),
            )
            .await;
            assert_denied(status, path);
        }
    }

    observer.deny.store(false, Ordering::SeqCst);
    for (path, body) in &reads {
        let status = post(
            &app,
            &format!("/collections/{COLLECTION}/{path}"),
            body.clone(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{path} after the denials");
    }
}

/// A denied read answers the denial error's own status and code on every
/// search endpoint — dense, ids, text, hybrid, batch and multi-query alike —
/// where it used to answer `400`, `500` or a code-less `403` depending on the
/// endpoint. Here the policy denies with `VELES-013`, an engine-class error,
/// so every endpoint answers `500` with that code.
#[tokio::test]
async fn read_gate_denials_answer_the_denial_errors_status_everywhere() {
    let dir = TempDir::new().expect("test: dir");
    let observer = Arc::new(SwitchableDenyObserver::default());
    let app =
        create_test_app_with_observer(&dir, Arc::clone(&observer) as Arc<dyn DatabaseObserver>);
    let status = post(
        &app,
        "/collections",
        json!({"name": COLLECTION, "dimension": DIM, "metric": "cosine"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    observer.deny.store(true, Ordering::SeqCst);

    let reads = [
        ("search", json!({"vector": QUERY, "top_k": 1})),
        ("search/ids", json!({"vector": QUERY, "top_k": 1})),
        ("search/text", json!({"query": "hello", "top_k": 1})),
        (
            "search/hybrid",
            json!({"vector": QUERY, "query": "hello", "top_k": 1, "vector_weight": 0.5}),
        ),
        (
            "search/batch",
            json!({"searches": [{"vector": QUERY, "top_k": 1}]}),
        ),
        (
            "search/multi",
            json!({"vectors": [QUERY, QUERY], "top_k": 1}),
        ),
        (
            "search/multi/ids",
            json!({"vectors": [QUERY, QUERY], "top_k": 1}),
        ),
    ];
    for (path, body) in &reads {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/collections/{COLLECTION}/{path}"))
                    .header("Content-Type", "application/json")
                    .body(Body::from(body.to_string()))
                    .expect("test: build POST request"),
            )
            .await
            .expect("test: POST request");
        assert_eq!(
            response.status(),
            StatusCode::INTERNAL_SERVER_ERROR,
            "{path}"
        );
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("test: read body");
        let json: Value = serde_json::from_slice(&bytes).expect("test: JSON body");
        assert_eq!(json["code"], "VELES-013", "{path}: {json}");
    }
}

async fn put(app: &axum::Router, uri: &str, body: Value) -> StatusCode {
    app.clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(uri)
                .header("Content-Type", "application/json")
                .body(Body::from(body.to_string()))
                .expect("test: build PUT request"),
        )
        .await
        .expect("test: PUT request")
        .status()
}

fn assert_denied(status: StatusCode, what: &str) {
    assert!(!status.is_success(), "{what} denied read, got {status}");
}

/// End-to-end proof that the CORE-2 read gate also covers the plain REST
/// graph endpoints, not just `/search*`, `MATCH`, and the embedding
/// `/graph/search` — see `graph_read_preamble` in `handlers/graph/handlers.rs`.
/// Before the fix this test guards, `graph_preamble` never consulted the
/// observer at all, so a denied principal could still list nodes, dump edges,
/// read node payloads, and traverse the graph through these endpoints while
/// `/search` correctly refused them.
#[tokio::test]
async fn read_gate_denies_rest_graph_reads_end_to_end() {
    const GRAPH: &str = "observed_graph";
    let dir = TempDir::new().expect("test: dir");
    let app = create_test_app_with_observer(&dir, Arc::new(DenyingReadObserver));

    // Writes are not read-gated: collection creation and seeding an edge must
    // still succeed under a deny-all observer.
    let status = post(
        &app,
        "/collections",
        json!({"name": GRAPH, "collection_type": "graph"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "create graph collection");

    // add_edge requires both endpoints to already have a stored payload
    // (VELES-022 NodeNotFound otherwise) — seed nodes 1 and 2 first.
    for node_id in [1, 2] {
        let uri = format!("/collections/{GRAPH}/graph/nodes/{node_id}/payload");
        let status = put(&app, &uri, json!({"payload": {}})).await;
        assert_eq!(
            status,
            StatusCode::NO_CONTENT,
            "seed node {node_id} payload must not be read-gated"
        );
    }

    let status = post(
        &app,
        &format!("/collections/{GRAPH}/graph/edges"),
        json!({"id": 1, "source": 1, "target": 2, "label": "KNOWS"}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "add_edge must not be read-gated"
    );

    // Every plain REST graph read must be refused end-to-end.
    assert_denied(
        get(
            &app,
            &format!("/collections/{GRAPH}/graph/edges?label=KNOWS"),
        )
        .await,
        "get_edges",
    );
    assert_denied(
        post(
            &app,
            &format!("/collections/{GRAPH}/graph/traverse"),
            json!({"source": 1, "strategy": "bfs"}),
        )
        .await,
        "traverse_graph",
    );
    assert_denied(
        get(&app, &format!("/collections/{GRAPH}/graph/nodes/1/degree")).await,
        "get_node_degree",
    );
    assert_denied(
        get(&app, &format!("/collections/{GRAPH}/graph/edges/count")).await,
        "get_edge_count",
    );
    assert_denied(
        get(&app, &format!("/collections/{GRAPH}/graph/nodes")).await,
        "list_nodes",
    );
    assert_denied(
        get(&app, &format!("/collections/{GRAPH}/graph/nodes/1/edges")).await,
        "get_node_edges",
    );
    assert_denied(
        get(&app, &format!("/collections/{GRAPH}/graph/nodes/1/payload")).await,
        "get_node_payload",
    );
    assert_denied(
        post(
            &app,
            &format!("/collections/{GRAPH}/graph/traverse/parallel"),
            json!({"sources": [1]}),
        )
        .await,
        "traverse_parallel",
    );
}
