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

use std::sync::atomic::{AtomicUsize, Ordering};
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
