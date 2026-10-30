//! The two upload routes that do not take JSON refuse any `Content-Type`
//! but the one they document, with `415 Unsupported Media Type`.
//!
//! Why it matters: a JSON route is out of reach of a cross-origin page,
//! because `application/json` makes a browser send a CORS preflight, which
//! the server's closed default refuses. These two routes read their body
//! raw, so before this guard a page on any site could `fetch(..., { mode:
//! 'no-cors', body })` with `text/plain` — a "simple" request, sent without a
//! preflight — and write points through a server it could not even read.
//! Requiring `application/x-ndjson` / `application/octet-stream`, which no
//! simple request can carry, closes that.

mod common;

use axum::{
    body::Body,
    http::{Request, StatusCode},
    Router,
};
use serde_json::json;
use tempfile::TempDir;
use tower::ServiceExt;

async fn app_with_collection(temp_dir: &TempDir) -> Router {
    let app = common::create_test_app(temp_dir);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({ "name": "c", "dimension": 2, "metric": "cosine" }).to_string(),
                ))
                .expect("test: build create"),
        )
        .await
        .expect("test: create");
    assert_eq!(response.status(), StatusCode::CREATED);
    app
}

async fn post(app: &Router, uri: &str, content_type: Option<&str>, body: &[u8]) -> StatusCode {
    let mut builder = Request::builder().method("POST").uri(uri);
    if let Some(content_type) = content_type {
        builder = builder.header("Content-Type", content_type);
    }
    app.clone()
        .oneshot(
            builder
                .body(Body::from(body.to_vec()))
                .expect("test: build"),
        )
        .await
        .expect("test: request")
        .status()
}

const NDJSON: &[u8] = b"{\"id\":1,\"vector\":[1.0,0.0]}\n";

#[tokio::test]
async fn test_stream_route_refuses_types_a_page_can_send_without_preflight() {
    let temp_dir = TempDir::new().expect("test: tempdir");
    let app = app_with_collection(&temp_dir).await;
    let uri = "/collections/c/points/stream";

    for refused in [
        Some("text/plain"),
        Some("text/plain;charset=UTF-8"),
        Some("application/x-www-form-urlencoded"),
        Some("multipart/form-data; boundary=x"),
        None,
    ] {
        assert_eq!(
            post(&app, uri, refused, NDJSON).await,
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "{refused:?}"
        );
    }
    // The documented type, with or without parameters, still works.
    for accepted in [
        "application/x-ndjson",
        "application/x-ndjson; charset=utf-8",
    ] {
        assert_eq!(
            post(&app, uri, Some(accepted), NDJSON).await,
            StatusCode::OK,
            "{accepted}"
        );
    }
}

#[tokio::test]
async fn test_raw_route_refuses_anything_but_octet_stream() {
    let temp_dir = TempDir::new().expect("test: tempdir");
    let app = app_with_collection(&temp_dir).await;
    let uri = "/collections/c/points/raw";

    for refused in [Some("text/plain"), Some("application/json"), None] {
        assert_eq!(
            post(&app, uri, refused, b"VRB1").await,
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "{refused:?}"
        );
    }
    // With the documented type the body reaches the decoder, which refuses
    // this truncated one on its own terms.
    assert_eq!(
        post(&app, uri, Some("application/octet-stream"), b"VRB1").await,
        StatusCode::BAD_REQUEST
    );
}
