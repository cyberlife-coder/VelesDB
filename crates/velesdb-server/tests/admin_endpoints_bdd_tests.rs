#![allow(clippy::doc_markdown)]
//! BDD tests for the maintenance and bulk-ops endpoints introduced in
//! PR #648:
//!
//! - `POST /collections/{name}/points/delete` — bulk delete by id
//! - `POST /collections/{name}/vacuum`         — HNSW index vacuum
//! - `POST /collections/{name}/compact`        — storage compaction
//!
//! Coverage per `.claude/rules/bdd-testing.md`:
//! - Nominal (~60%): happy paths, end-to-end behaviour observable from
//!   the REST surface.
//! - Edge (~20%): boundary conditions (empty payload, max-batch size,
//!   collection with no deletions to compact).
//! - Negative (~20%): unknown collection, oversized batch, malformed
//!   JSON. Each must produce the documented HTTP status code.
//!
//! Tests build a router via `common::create_test_app` and exercise the
//! endpoints with `tower::ServiceExt::oneshot` requests, asserting
//! status codes and JSON response shape — no internal state inspection.

mod common;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use common::create_test_app;
use serde_json::{json, Value};
use tempfile::TempDir;
use tower::ServiceExt;

const TEST_COLLECTION: &str = "admin_endpoints_bdd";
const DIM: usize = 4;

/// Bootstrap a vector collection and seed it with `n` deterministic points.
async fn seed_collection(app: axum::Router, n: usize) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "name": TEST_COLLECTION,
                        "dimension": DIM,
                        "metric": "cosine"
                    })
                    .to_string(),
                ))
                .expect("test: build create request"),
        )
        .await
        .expect("test: collection create request");
    assert_eq!(
        response.status(),
        StatusCode::CREATED,
        "test setup: failed to create collection"
    );

    if n == 0 {
        return;
    }
    let points: Vec<Value> = (0..n)
        .map(|i| {
            #[allow(clippy::cast_precision_loss)]
            let v = (i as f32) / 100.0;
            json!({
                "id": u64::try_from(i).expect("test: idx fits in u64") + 1,
                "vector": vec![v; DIM],
                "payload": { "idx": i }
            })
        })
        .collect();
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/collections/{TEST_COLLECTION}/points"))
                .header("Content-Type", "application/json")
                .body(Body::from(json!({ "points": points }).to_string()))
                .expect("test: build upsert request"),
        )
        .await
        .expect("test: upsert request");
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "test setup: failed to seed points"
    );
}

async fn read_json(response: axum::response::Response) -> Value {
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("test: read body");
    serde_json::from_slice(&body).expect("test: response is valid JSON")
}

// ---------------------------------------------------------------------
// /points/delete — bulk_delete_points
// ---------------------------------------------------------------------

#[tokio::test]
async fn bulk_delete_nominal_returns_200_with_deleted_count() {
    let temp_dir = TempDir::new().expect("test: temp dir");
    let app = create_test_app(&temp_dir);
    seed_collection(app.clone(), 5).await;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/collections/{TEST_COLLECTION}/points/delete"))
                .header("Content-Type", "application/json")
                .body(Body::from(json!({ "ids": [1, 2, 3] }).to_string()))
                .expect("test: build delete request"),
        )
        .await
        .expect("test: delete request");
    assert_eq!(response.status(), StatusCode::OK);

    let json = read_json(response).await;
    assert_eq!(json["deleted_count"], 3);
    assert_eq!(json["collection"], TEST_COLLECTION);
}

#[tokio::test]
async fn bulk_delete_empty_payload_returns_200_noop() {
    // Documented behaviour: empty `ids: []` is a no-op (200, count=0).
    // See `bulk_delete_points` rustdoc — idempotent batch semantics.
    let temp_dir = TempDir::new().expect("test: temp dir");
    let app = create_test_app(&temp_dir);
    seed_collection(app.clone(), 3).await;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/collections/{TEST_COLLECTION}/points/delete"))
                .header("Content-Type", "application/json")
                .body(Body::from(json!({ "ids": [] }).to_string()))
                .expect("test: build delete request"),
        )
        .await
        .expect("test: delete request");
    assert_eq!(response.status(), StatusCode::OK);

    let json = read_json(response).await;
    assert_eq!(json["deleted_count"], 0);
}

#[tokio::test]
async fn bulk_delete_unknown_ids_silently_skipped() {
    // Idempotent: deleting non-existent IDs returns 200, count = batch size.
    let temp_dir = TempDir::new().expect("test: temp dir");
    let app = create_test_app(&temp_dir);
    seed_collection(app.clone(), 2).await;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/collections/{TEST_COLLECTION}/points/delete"))
                .header("Content-Type", "application/json")
                .body(Body::from(json!({ "ids": [999, 1000] }).to_string()))
                .expect("test: build delete request"),
        )
        .await
        .expect("test: delete request");
    assert_eq!(response.status(), StatusCode::OK);
    let json = read_json(response).await;
    assert_eq!(json["deleted_count"], 2);
}

#[tokio::test]
async fn bulk_delete_oversized_batch_returns_400() {
    // Negative: > MAX_BULK_DELETE_SIZE (10_000) must be 400.
    let temp_dir = TempDir::new().expect("test: temp dir");
    let app = create_test_app(&temp_dir);
    seed_collection(app.clone(), 1).await;

    let oversized: Vec<u64> = (1..=10_001).collect();
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/collections/{TEST_COLLECTION}/points/delete"))
                .header("Content-Type", "application/json")
                .body(Body::from(json!({ "ids": oversized }).to_string()))
                .expect("test: build delete request"),
        )
        .await
        .expect("test: delete request");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn bulk_delete_unknown_collection_returns_404() {
    // Negative: ghost collection must be 404.
    let temp_dir = TempDir::new().expect("test: temp dir");
    let app = create_test_app(&temp_dir);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections/ghost_collection/points/delete")
                .header("Content-Type", "application/json")
                .body(Body::from(json!({ "ids": [1, 2] }).to_string()))
                .expect("test: build delete request"),
        )
        .await
        .expect("test: delete request");
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn bulk_delete_malformed_payload_returns_400() {
    // Negative: missing `ids` field must be a deserialisation failure (4xx).
    let temp_dir = TempDir::new().expect("test: temp dir");
    let app = create_test_app(&temp_dir);
    seed_collection(app.clone(), 1).await;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/collections/{TEST_COLLECTION}/points/delete"))
                .header("Content-Type", "application/json")
                .body(Body::from(json!({ "wrong_field": [1, 2] }).to_string()))
                .expect("test: build delete request"),
        )
        .await
        .expect("test: delete request");
    assert!(
        response.status().is_client_error(),
        "expected 4xx for malformed payload, got {}",
        response.status()
    );
}

// ---------------------------------------------------------------------
// /vacuum — vacuum_collection (alias of /index/rebuild, by design)
// ---------------------------------------------------------------------

#[tokio::test]
async fn vacuum_nominal_returns_200_with_compacted_count() {
    let temp_dir = TempDir::new().expect("test: temp dir");
    let app = create_test_app(&temp_dir);
    seed_collection(app.clone(), 4).await;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/collections/{TEST_COLLECTION}/vacuum"))
                .header("Content-Type", "application/json")
                .body(Body::empty())
                .expect("test: build vacuum request"),
        )
        .await
        .expect("test: vacuum request");
    assert_eq!(response.status(), StatusCode::OK);

    let json = read_json(response).await;
    assert_eq!(json["message"], "Index vacuumed");
    assert_eq!(json["collection"], TEST_COLLECTION);
    assert!(
        json["compacted_entries"].is_number(),
        "compacted_entries must be a number, got {:?}",
        json["compacted_entries"]
    );
}

#[tokio::test]
async fn vacuum_empty_collection_returns_200() {
    // Edge: vacuuming a collection with 0 vectors must succeed (no-op).
    let temp_dir = TempDir::new().expect("test: temp dir");
    let app = create_test_app(&temp_dir);
    seed_collection(app.clone(), 0).await;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/collections/{TEST_COLLECTION}/vacuum"))
                .body(Body::empty())
                .expect("test: build vacuum request"),
        )
        .await
        .expect("test: vacuum request");
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn vacuum_unknown_collection_returns_404() {
    let temp_dir = TempDir::new().expect("test: temp dir");
    let app = create_test_app(&temp_dir);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections/ghost_collection/vacuum")
                .body(Body::empty())
                .expect("test: build vacuum request"),
        )
        .await
        .expect("test: vacuum request");
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

// ---------------------------------------------------------------------
// /compact — compact_collection
// ---------------------------------------------------------------------

#[tokio::test]
async fn compact_nominal_returns_200_with_bytes_reclaimed() {
    let temp_dir = TempDir::new().expect("test: temp dir");
    let app = create_test_app(&temp_dir);
    seed_collection(app.clone(), 6).await;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/collections/{TEST_COLLECTION}/compact"))
                .body(Body::empty())
                .expect("test: build compact request"),
        )
        .await
        .expect("test: compact request");
    assert_eq!(response.status(), StatusCode::OK);

    let json = read_json(response).await;
    assert_eq!(json["message"], "Storage compacted");
    assert_eq!(json["collection"], TEST_COLLECTION);
    assert!(
        json["bytes_reclaimed"].is_number(),
        "bytes_reclaimed must be a number, got {:?}",
        json["bytes_reclaimed"]
    );
}

#[tokio::test]
async fn compact_empty_collection_returns_200() {
    // Edge: compacting an empty collection is a valid no-op.
    let temp_dir = TempDir::new().expect("test: temp dir");
    let app = create_test_app(&temp_dir);
    seed_collection(app.clone(), 0).await;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/collections/{TEST_COLLECTION}/compact"))
                .body(Body::empty())
                .expect("test: build compact request"),
        )
        .await
        .expect("test: compact request");
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn compact_unknown_collection_returns_404() {
    let temp_dir = TempDir::new().expect("test: temp dir");
    let app = create_test_app(&temp_dir);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections/ghost_collection/compact")
                .body(Body::empty())
                .expect("test: build compact request"),
        )
        .await
        .expect("test: compact request");
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

// ---------------------------------------------------------------------
// Cross-endpoint sanity: delete -> vacuum -> compact lifecycle
// ---------------------------------------------------------------------

#[tokio::test]
async fn lifecycle_delete_then_vacuum_then_compact_succeeds() {
    let temp_dir = TempDir::new().expect("test: temp dir");
    let app = create_test_app(&temp_dir);
    seed_collection(app.clone(), 8).await;

    // Delete half the points.
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/collections/{TEST_COLLECTION}/points/delete"))
                .header("Content-Type", "application/json")
                .body(Body::from(json!({ "ids": [1, 2, 3, 4] }).to_string()))
                .expect("test: build delete request"),
        )
        .await
        .expect("test: delete request");
    assert_eq!(response.status(), StatusCode::OK);

    // Vacuum the (now half-empty) index.
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/collections/{TEST_COLLECTION}/vacuum"))
                .body(Body::empty())
                .expect("test: build vacuum request"),
        )
        .await
        .expect("test: vacuum request");
    assert_eq!(response.status(), StatusCode::OK);

    // Compact the storage.
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/collections/{TEST_COLLECTION}/compact"))
                .body(Body::empty())
                .expect("test: build compact request"),
        )
        .await
        .expect("test: compact request");
    assert_eq!(response.status(), StatusCode::OK);
}
